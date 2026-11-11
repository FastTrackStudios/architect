//! The engine answers [`Files`](crate::agent::Files).

use std::collections::BTreeMap;
use std::str::FromStr as _;

use architect::blob::Sha256;
use uuid::Uuid;

use super::engine::{Engine, EngineError};
use super::manifest::{Tree, doc_id_for};
use crate::agent::{AgentError, Files};
use crate::copies::{CopyReport, HolderStatus, Locations, Policy, ScrubReport, Violation};
use crate::id::VersionId;
use crate::rendition::Peaks;
use crate::roots::{
    Asset, Checkpoint, DriveStatus, FileEntry, FileVersion, Hit, RootStatus, VersionBytes,
};

impl From<EngineError> for AgentError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::NoRoot(r) => Self::NotFound(r),
            EngineError::NoVersion => Self::NotFound("the version".to_owned()),
            EngineError::NotSynced(paths) => Self::NotSynced(paths.join(", ")),
            EngineError::BelowPolicy(paths) => Self::BelowPolicy(first_few(&paths)),
            EngineError::Io(why) => Self::Refused(why),
            EngineError::NoRoom(why) | EngineError::TooLarge(why) => Self::NoRoom(why),
            e @ EngineError::Changed(_) => Self::NotSynced(e.to_string()),
        }
    }
}

/// `a, b, c and 24 more files`: a refusal stays readable at any size.
fn first_few(paths: &[String]) -> String {
    const SHOWN: usize = 3;
    let head = paths
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    match paths.len().saturating_sub(SHOWN) {
        0 => head,
        more => format!("{head} and {more} more files"),
    }
}

impl Engine {
    /// The manifest id `root` names: an id, or a place this device knows.
    ///
    /// # Errors
    ///
    /// Neither.
    pub fn resolve_root(&self, root: &str) -> Result<Uuid, AgentError> {
        let id = Uuid::from_str(root).unwrap_or_else(|_| doc_id_for(root.trim_matches('/')));
        if self.knows(id) {
            Ok(id)
        } else {
            Err(AgentError::NotFound(root.to_owned()))
        }
    }

    /// One folder of a root's live files.
    #[must_use]
    pub fn entries(&self, tree: &Tree, path: &str) -> Vec<FileEntry> {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{}/", path.trim_matches('/'))
        };
        let mut dirs: BTreeMap<String, (u64, i64)> = BTreeMap::new();
        let mut out = Vec::new();
        for (p, v) in tree.live() {
            let Some(rest) = p.strip_prefix(&prefix) else {
                continue;
            };
            match rest.split_once('/') {
                Some((dir, _)) => {
                    let e = dirs.entry(dir.to_owned()).or_default();
                    e.0 = e.0.saturating_add(v.size);
                    e.1 = e.1.max(v.at_us);
                }
                None => out.push(FileEntry {
                    name: rest.to_owned(),
                    here: v
                        .content
                        .as_deref()
                        .and_then(|c| Sha256::from_hex(c).ok())
                        .is_some_and(|h| self.holds(&h)),
                    path: p.clone(),
                    dir: false,
                    size: v.size,
                    content: v.content.clone(),
                    version: Some(v.id),
                    at_us: v.at_us,
                }),
            }
        }
        let mut folders: Vec<FileEntry> = dirs
            .into_iter()
            .map(|(name, (size, at_us))| FileEntry {
                path: format!("{prefix}{name}"),
                name,
                dir: true,
                size,
                content: None,
                version: None,
                at_us,
                here: false,
            })
            .collect();
        folders.append(&mut out);
        folders
    }
}

fn version_id(raw: &str) -> Result<VersionId, AgentError> {
    VersionId::from_str(raw).map_err(|_| AgentError::NotFound(raw.to_owned()))
}

impl Files for Engine {
    async fn roots(&self) -> Result<Vec<RootStatus>, AgentError> {
        Ok(self.status().await)
    }

    async fn browse(&self, root: String, path: String) -> Result<Vec<FileEntry>, AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(self.entries(&self.tree(id).await?, &path))
    }

    async fn pin(&self, root: String, path: Option<String>) -> Result<String, AgentError> {
        let id = self.resolve_root(&root)?;
        let at = Self::pin(self, id, path.filter(|p| !p.is_empty()).map(Into::into)).await?;
        Ok(at.display().to_string())
    }

    async fn unpin(&self, root: String) -> Result<(), AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(Self::unpin(self, id).await?)
    }

    async fn history(&self, root: String, path: String) -> Result<Vec<FileVersion>, AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(self.tree(id).await?.history(&path).to_vec())
    }

    async fn held(&self, root: String, path: String) -> Result<Vec<VersionBytes>, AgentError> {
        let id = self.resolve_root(&root)?;
        let versions = self.tree(id).await?.history(&path).to_vec();
        let mut out = Vec::new();
        for v in versions {
            if let Some(content) = &v.content {
                out.push(self.version_bytes(v.id, content).await);
            }
        }
        Ok(out)
    }

    async fn restore(
        &self,
        root: String,
        path: String,
        version: String,
    ) -> Result<FileVersion, AgentError> {
        let id = self.resolve_root(&root)?;
        let restored = Self::restore(self, id, &path, version_id(&version)?).await?;
        self.pass(id).await;
        Ok(restored)
    }

    async fn checkpoint(&self, root: String, name: String) -> Result<Checkpoint, AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(self.make_checkpoint(id, &name).await?)
    }

    async fn checkpoints(&self, root: String) -> Result<Vec<Checkpoint>, AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(Self::checkpoints(self, id).await?)
    }

    async fn restore_checkpoint(
        &self,
        root: String,
        checkpoint: String,
    ) -> Result<u32, AgentError> {
        let id = self.resolve_root(&root)?;
        let changed = Self::restore_checkpoint(self, id, version_id(&checkpoint)?).await?;
        self.pass(id).await;
        Ok(changed)
    }

    async fn assets(&self, root: String) -> Result<Vec<Asset>, AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(Self::assets(self, id).await?)
    }

    async fn search(&self, query: String) -> Result<Vec<Hit>, AgentError> {
        Ok(Self::search(self, &query).await)
    }

    async fn locations(&self, root: String, path: String) -> Result<Locations, AgentError> {
        if root.trim().is_empty() {
            let hash = path.trim().trim_start_matches("sha:").to_lowercase();
            if Sha256::from_hex(&hash).is_err() {
                return Err(AgentError::NotFound(path));
            }
            return Ok(Self::locations(self, &hash).await);
        }
        let id = self.resolve_root(&root)?;
        Ok(self.locations_of(id, path.trim_matches('/')).await?)
    }

    async fn copies(&self, scope: String) -> Result<CopyReport, AgentError> {
        self.refresh_registry().await;
        self.refresh_ledgers().await;
        Ok(self.copy_report(scope.trim_matches('/')).await)
    }

    async fn policy_set(
        &self,
        scope: String,
        min_verified: u32,
        min_sites: u32,
    ) -> Result<Policy, AgentError> {
        let scope = scope.trim_matches('/');
        let policy = (min_verified > 0).then_some(Policy {
            min_verified,
            min_sites: min_sites.max(1),
        });
        self.set_policy(scope, policy).await?;
        let org = scope.split('/').next().unwrap_or(scope);
        Ok(self.policy_for(org, scope))
    }

    async fn policy_check(&self, scope: String) -> Result<Vec<Violation>, AgentError> {
        self.refresh_ledgers().await;
        Ok(self.copy_report(scope.trim_matches('/')).await.violations)
    }

    async fn scrub(&self, scope: String) -> Result<ScrubReport, AgentError> {
        let scope = scope.trim().trim_matches('/');
        let mut report = if scope.is_empty() {
            Self::scrub(self, None, u64::MAX).await
        } else {
            Self::scrub(self, Some(scope), u64::MAX).await
        };
        let hash = (Sha256::from_hex(scope).is_ok()).then_some(scope);
        let c = self.challenge(64, hash).await;
        report.challenged = c.challenged;
        report.passed = c.passed;
        report.failed = c.failed;
        Ok(report)
    }

    async fn evict(&self, root: String, path: String) -> Result<u32, AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(Self::evict(self, id, path.trim_matches('/')).await?)
    }

    async fn holders(&self) -> Result<Vec<HolderStatus>, AgentError> {
        Ok(Self::holders(self).await)
    }

    async fn peaks(&self, root: String, path: String) -> Result<Peaks, AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(Self::peaks(self, id, &path).await?)
    }

    async fn drive(&self) -> Result<DriveStatus, AgentError> {
        let (mount, online) = self.drive_info();
        let roots = self.status().await;
        Ok(DriveStatus {
            mounted: mount.as_deref().is_some_and(mounted_at),
            mount: mount.map(|m| m.display().to_string()),
            syncing: u32::try_from(roots.iter().filter(|r| r.pinned && !r.synced).count())
                .unwrap_or(u32::MAX),
            offline: !online,
            cache_bytes: dir_size(self.store.local.root()),
        })
    }
}

/// Whether a FUSE mount is at `at` (Linux: `/proc/self/mountinfo`).
fn mounted_at(at: &std::path::Path) -> bool {
    let target = at.to_string_lossy().replace(' ', "\\040");
    std::fs::read_to_string("/proc/self/mountinfo").is_ok_and(|info| {
        info.lines()
            .any(|l| l.split(' ').nth(4) == Some(target.as_str()) && l.contains(" - fuse"))
    })
}

fn dir_size(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => dir_size(&e.path()),
                Ok(_) => e.metadata().map_or(0, |m| m.len()),
                Err(_) => 0,
            })
            .fold(0_u64, u64::saturating_add)
    })
}
