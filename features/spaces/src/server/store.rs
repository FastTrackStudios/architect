//! Where an org's spaces and their audit trail are kept.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use architect_orgs::OrgId;
use tokio::io::AsyncWriteExt as _;

use crate::model::{OrgSpaces, SpaceAudit, SpaceId, SpacesFault};

/// A boxed answer from a [`SpaceStore`].
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, SpacesFault>> + Send + 'a>>;

/// Keeps every org's spaces (written whole after each change) and an
/// append-only trail of the changes.
///
/// [`crate::server::Spaces`] reads everything once at open and answers
/// checks from memory, so a store is only asked to load, save and
/// append.
pub trait SpaceStore: Send + Sync {
    /// Every org's spaces.
    fn load(&self) -> StoreFuture<'_, Vec<(OrgId, OrgSpaces)>>;

    /// Replace `org`'s spaces with `spaces`.
    fn save<'a>(&'a self, org: &'a OrgId, spaces: &'a OrgSpaces) -> StoreFuture<'a, ()>;

    /// Append one change to `org`'s trail.
    fn record<'a>(&'a self, org: &'a OrgId, entry: &'a SpaceAudit) -> StoreFuture<'a, ()>;

    /// `org`'s trail for one space, oldest first.
    fn audit<'a>(&'a self, org: &'a OrgId, space: &'a SpaceId) -> StoreFuture<'a, Vec<SpaceAudit>>;
}

/// In memory: tests, and a server that keeps spaces elsewhere.
#[derive(Clone, Default)]
pub struct MemoryStore {
    orgs: Arc<Mutex<HashMap<OrgId, (OrgSpaces, Vec<SpaceAudit>)>>>,
}

impl MemoryStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn with<T>(&self, f: impl FnOnce(&mut HashMap<OrgId, (OrgSpaces, Vec<SpaceAudit>)>) -> T) -> T {
        f(&mut self.orgs.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl SpaceStore for MemoryStore {
    fn load(&self) -> StoreFuture<'_, Vec<(OrgId, OrgSpaces)>> {
        let all = self.with(|orgs| {
            orgs.iter()
                .map(|(org, (spaces, _))| (org.clone(), spaces.clone()))
                .collect()
        });
        Box::pin(async move { Ok(all) })
    }

    fn save<'a>(&'a self, org: &'a OrgId, spaces: &'a OrgSpaces) -> StoreFuture<'a, ()> {
        self.with(|orgs| orgs.entry(org.clone()).or_default().0 = spaces.clone());
        Box::pin(async { Ok(()) })
    }

    fn record<'a>(&'a self, org: &'a OrgId, entry: &'a SpaceAudit) -> StoreFuture<'a, ()> {
        self.with(|orgs| orgs.entry(org.clone()).or_default().1.push(entry.clone()));
        Box::pin(async { Ok(()) })
    }

    fn audit<'a>(&'a self, org: &'a OrgId, space: &'a SpaceId) -> StoreFuture<'a, Vec<SpaceAudit>> {
        let trail = self.with(|orgs| {
            orgs.get(org)
                .map(|(_, trail)| of_space(trail.iter().cloned(), space))
                .unwrap_or_default()
        });
        Box::pin(async move { Ok(trail) })
    }
}

fn of_space(trail: impl Iterator<Item = SpaceAudit>, space: &SpaceId) -> Vec<SpaceAudit> {
    trail.filter(|e| e.space.as_ref() == Some(space)).collect()
}

/// On disk, beside an org's other data: `<dir>/org/<id>/spaces/` holds
/// `state.json` (replaced atomically on every change) and `audit.jsonl`
/// (one change per line, appended).
#[derive(Clone, Debug)]
pub struct FileStore {
    dir: PathBuf,
}

const STATE_FILE: &str = "state.json";
const AUDIT_FILE: &str = "audit.jsonl";

impl FileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn org_dir(&self, org: &OrgId) -> PathBuf {
        self.dir.join(org.path("spaces"))
    }
}

fn io(e: impl std::fmt::Display) -> SpacesFault {
    SpacesFault::Internal(e.to_string())
}

impl SpaceStore for FileStore {
    fn load(&self) -> StoreFuture<'_, Vec<(OrgId, OrgSpaces)>> {
        Box::pin(async move {
            let mut all = Vec::new();
            let mut orgs = match tokio::fs::read_dir(self.dir.join("org")).await {
                Ok(dir) => dir,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(all),
                Err(e) => return Err(io(e)),
            };
            while let Some(entry) = orgs.next_entry().await.map_err(io)? {
                let Some(id) = entry.file_name().to_str().map(OrgId::new) else {
                    continue;
                };
                let path = self.org_dir(&id).join(STATE_FILE);
                match tokio::fs::read_to_string(&path).await {
                    Ok(json) => {
                        let spaces = facet_json::from_str(&json)
                            .map_err(|e| io(format!("{}: {e}", path.display())))?;
                        all.push((id, spaces));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io(e)),
                }
            }
            Ok(all)
        })
    }

    fn save<'a>(&'a self, org: &'a OrgId, spaces: &'a OrgSpaces) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let json = facet_json::to_string(spaces).map_err(io)?;
            let dir = self.org_dir(org);
            tokio::fs::create_dir_all(&dir).await.map_err(io)?;
            let tmp = dir.join(format!("{STATE_FILE}.tmp"));
            tokio::fs::write(&tmp, json).await.map_err(io)?;
            tokio::fs::rename(&tmp, dir.join(STATE_FILE))
                .await
                .map_err(io)
        })
    }

    fn record<'a>(&'a self, org: &'a OrgId, entry: &'a SpaceAudit) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut line = facet_json::to_string(entry).map_err(io)?;
            line.push('\n');
            let dir = self.org_dir(org);
            tokio::fs::create_dir_all(&dir).await.map_err(io)?;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(AUDIT_FILE))
                .await
                .map_err(io)?;
            file.write_all(line.as_bytes()).await.map_err(io)?;
            file.flush().await.map_err(io)
        })
    }

    fn audit<'a>(&'a self, org: &'a OrgId, space: &'a SpaceId) -> StoreFuture<'a, Vec<SpaceAudit>> {
        Box::pin(async move {
            let text = match tokio::fs::read_to_string(self.org_dir(org).join(AUDIT_FILE)).await {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(e) => return Err(io(e)),
            };
            let entries = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| facet_json::from_str::<SpaceAudit>(l).map_err(io))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(of_space(entries.into_iter(), space))
        })
    }
}
