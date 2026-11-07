//! The engine answers [`Files`](crate::agent::Files).

use std::collections::BTreeMap;
use std::str::FromStr as _;

use architect::blob::Sha256;
use uuid::Uuid;

use super::engine::{Engine, EngineError};
use super::manifest::{Tree, doc_id_for};
use crate::agent::{AgentError, Files};
use crate::id::VersionId;
use crate::rendition::Peaks;
use crate::roots::{Asset, Checkpoint, FileEntry, FileVersion, Hit, RootStatus};

impl From<EngineError> for AgentError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::NoRoot(r) => Self::NotFound(r),
            EngineError::NoVersion => Self::NotFound("the version".to_owned()),
            EngineError::NotSynced(paths) => Self::NotSynced(paths.join(", ")),
            EngineError::Io(why) => Self::Refused(why),
        }
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

    async fn peaks(&self, root: String, path: String) -> Result<Peaks, AgentError> {
        let id = self.resolve_root(&root)?;
        Ok(Self::peaks(self, id, &path).await?)
    }
}
