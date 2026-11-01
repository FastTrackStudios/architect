//! The service, for one principal in one org: what every call does.

use std::sync::Arc;

use architect::EventSink;
use architect::blob::{BlobStore as _, Sha256};
use architect::vox::{Rx, Tx};
use architect_orgs::OrgId;
use architect_permissions::Principal;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

use super::state::{Batch, Node, RootState, Stamp, State, Upload, add_range, root_entry};
use super::{Actor, FilesServer, OrgFiles, io, now_us};
use crate::access::{Access, Capability, Effective, Grant, canonical};
use crate::client::FRAME;
use crate::error::FilesFault;
use crate::id::{ContentId, GrantId, LabelId, RootId, UploadId, VersionId};
use crate::model::{
    ByteRange, Entry, EntryKind, Expect, FilesEvent, OnConflict, ReadDone, ReadRequest, Received,
    Relocation, Root, UploadFrame, UploadPlan, UploadProgress, UploadSpec, Usage, Version,
    VersionLabel, WriteReceipt,
};
use crate::path::RootPath;
use crate::service::{FilesService, FilesServiceStreamSource};

/// [`FilesService`] for one principal in one org
/// ([`FilesServer::acting`]).
#[derive(Clone)]
pub struct Acting {
    server: FilesServer,
    org: OrgId,
    who: Principal,
}

impl Acting {
    pub(super) const fn new(server: FilesServer, org: OrgId, who: Principal) -> Self {
        Self { server, org, who }
    }

    async fn ctx(&self) -> Result<(Arc<OrgFiles>, Actor), FilesFault> {
        let actor = self.server.actor(&self.org, &self.who)?;
        Ok((self.server.org(&self.org).await?, actor))
    }

    /// Answer from the org's state as it is.
    async fn view<T>(
        &self,
        f: impl FnOnce(&State, &Actor) -> Result<T, FilesFault>,
    ) -> Result<T, FilesFault> {
        let (files, actor) = self.ctx().await?;
        let state = files.state.lock().await;
        f(&state, &actor)
    }

    /// Change the org's state: `f` works on a copy, which replaces the
    /// state (on disk, then in memory) only if `f` succeeds and the org's
    /// files still fit its quota; then its events go out.
    async fn mutate<T>(
        &self,
        f: impl FnOnce(&mut State, &Actor, &mut Stamp<'_>) -> Result<(T, Vec<FilesEvent>), FilesFault>,
    ) -> Result<T, FilesFault> {
        let (files, actor) = self.ctx().await?;
        let mut state = files.state.lock().await;
        let mut next = state.clone();
        let (out, events) = {
            let mut mint = VersionId::generate;
            let mut stamp = Stamp {
                now: now_us(),
                by: &actor.label,
                mint: &mut mint,
            };
            f(&mut next, &actor, &mut stamp)?
        };
        let (before, after) = (state.used(), next.used());
        if let Some(quota) = next.quota
            && after > quota
            && after > before
        {
            return Err(FilesFault::QuotaExceeded {
                used: before,
                quota,
                adding: after.saturating_sub(before),
            });
        }
        files.save(&next).await?;
        *state = next;
        drop(state);
        for event in events {
            files.hub.publish(event);
        }
        Ok(out)
    }

    /// Apply a tree batch to one root of `state`.
    fn batch<T>(
        state: &mut State,
        root: RootId,
        stamp: &mut Stamp<'_>,
        f: impl FnOnce(&mut Batch<'_, '_>) -> Result<T, FilesFault>,
    ) -> Result<(T, Vec<FilesEvent>), FilesFault> {
        let nodes = &mut state.root_mut(root)?.nodes;
        let mut batch = Batch::new(
            root,
            nodes,
            Stamp {
                now: stamp.now,
                by: stamp.by,
                mint: &mut *stamp.mint,
            },
        );
        let out = f(&mut batch)?;
        Ok((out, batch.events()))
    }

    /// The upload `id`, if it is this actor's.
    fn upload<'s>(state: &'s State, actor: &Actor, id: UploadId) -> Result<&'s Upload, FilesFault> {
        state
            .uploads
            .iter()
            .find(|u| u.id == id && u.by == actor.label)
            .ok_or(FilesFault::UploadNotFound(id))
    }

    /// Refuse a save `expect` does not hold for.
    fn check_expect(
        node: Option<&Node>,
        path: &RootPath,
        expect: Option<&Expect>,
    ) -> Result<(), FilesFault> {
        let current = node
            .filter(|n| n.is_live() && n.is_file())
            .and_then(|n| n.current())
            .and_then(|v| v.content.as_ref());
        match expect {
            Some(Expect::Absent) if node.is_some_and(Node::is_live) => Err(FilesFault::Stale {
                path: path.clone(),
                expected: "absent".into(),
            }),
            Some(Expect::Content(want)) if current != Some(want) => Err(FilesFault::Stale {
                path: path.clone(),
                expected: want.to_string(),
            }),
            None | Some(_) => Ok(()),
        }
    }

    /// Put an upload's staged bytes in the blob store; their content.
    async fn store_staged(
        &self,
        files: &OrgFiles,
        upload: &Upload,
    ) -> Result<ContentId, FilesFault> {
        let expected = upload
            .spec
            .content
            .as_ref()
            .map(|c| Sha256::from_hex(c.as_str()))
            .transpose()
            .map_err(|e| FilesFault::invalid(e.to_string()))?;
        if !upload.staged {
            // Nothing was sent: the org held these bytes already.
            let hash = expected.ok_or_else(|| {
                FilesFault::Internal("an unstaged upload names no content".into())
            })?;
            if !self.server.blobs().has(hash).await.map_err(io)? {
                return Err(FilesFault::Incomplete(upload.id));
            }
            return Ok(ContentId::new(hash.to_hex()));
        }
        let staged = files.staging(upload.id);
        let file = tokio::fs::File::open(&staged).await.map_err(io)?;
        let reader: architect::blob::Reader = Box::pin(file.take(upload.spec.size));
        let hash = self
            .server
            .blobs()
            .put(expected, reader)
            .await
            .map_err(|e| match e {
                architect::blob::BlobError::HashMismatch {
                    expected, actual, ..
                } => FilesFault::invalid(format!("the bytes sent are {actual}, not {expected}")),
                other => io(other),
            })?;
        Ok(ContentId::new(hash.to_hex()))
    }
}

impl FilesService for Acting {
    // ── Roots ──────────────────────────────────────────────────────

    async fn ensure_root(&self, dir: String, name: String) -> Result<Root, FilesFault> {
        let key = RootPath::parse(&dir)?;
        let name = name.trim().to_owned();
        if key.is_root() || name.is_empty() {
            return Err(FilesFault::invalid("a root needs a dir and a name"));
        }
        let created = self
            .mutate(|state, actor, stamp| {
                if let Some(existing) = state.roots.iter().find(|r| r.root.dir == key.as_str()) {
                    return Ok(((existing.root.clone(), false), Vec::new()));
                }
                if !actor.baseline.contains(&Capability::Write) {
                    return Err(FilesFault::denied("Write", key.clone()));
                }
                let root = Root {
                    id: RootId::generate(),
                    name,
                    dir: key.as_str().to_owned(),
                    created_by: stamp.by.to_owned(),
                    created_us: stamp.now,
                };
                state.roots.push(RootState {
                    root: root.clone(),
                    nodes: std::collections::BTreeMap::new(),
                });
                Ok(((root.clone(), true), vec![FilesEvent::RootCreated(root)]))
            })
            .await?;
        // An existing root is answered to anyone who can see it; one they
        // cannot is absent, like everything else they hold nothing in.
        let (root, made) = created;
        if made {
            return Ok(root);
        }
        self.root(root.id).await
    }

    async fn roots(&self) -> Result<Vec<Root>, FilesFault> {
        self.view(|state, actor| {
            let now = now_us();
            Ok(state
                .roots
                .iter()
                .filter(|r| actor.sees_root(state, r.root.id, now))
                .map(|r| r.root.clone())
                .collect())
        })
        .await
    }

    async fn root(&self, root_id: RootId) -> Result<Root, FilesFault> {
        self.view(|state, actor| {
            let root = state.root(root_id)?;
            if actor.sees_root(state, root_id, now_us()) {
                Ok(root.root.clone())
            } else {
                Err(FilesFault::RootNotFound(root_id))
            }
        })
        .await
    }

    async fn rename_root(&self, root_id: RootId, name: String) -> Result<Root, FilesFault> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(FilesFault::invalid("a root needs a name"));
        }
        self.mutate(|state, actor, _| {
            steward(state, actor, root_id)?;
            let root = &mut state.root_mut(root_id)?.root;
            root.name = name;
            Ok((root.clone(), vec![FilesEvent::RootRenamed(root.clone())]))
        })
        .await
    }

    async fn release_root(&self, root_id: RootId) -> Result<(), FilesFault> {
        let (files, _) = self.ctx().await?;
        let dropped = self
            .mutate(|state, actor, _| {
                steward(state, actor, root_id)?;
                state.roots.retain(|r| r.root.id != root_id);
                state.labels.retain(|l| l.root_id != root_id);
                state.grants.retain(|g| g.root_id != root_id);
                let (gone, kept) = std::mem::take(&mut state.uploads)
                    .into_iter()
                    .partition(|u| u.spec.root_id == root_id);
                state.uploads = kept;
                Ok((gone, vec![FilesEvent::RootReleased(root_id)]))
            })
            .await?;
        discard(&files, &dropped).await;
        Ok(())
    }

    async fn usage(&self) -> Result<Usage, FilesFault> {
        self.view(|state, actor| {
            if actor.baseline.is_empty() {
                return Err(FilesFault::denied("Read", RootPath::root()));
            }
            Ok(Usage {
                used: state.used(),
                quota: state.quota,
            })
        })
        .await
    }

    // ── The tree ───────────────────────────────────────────────────

    async fn browse(&self, root_id: RootId, path: RootPath) -> Result<Vec<Entry>, FilesFault> {
        let path = path.validate()?;
        self.view(|state, actor| {
            let nodes = &state.root(root_id)?.nodes;
            if !actor.at(state, root_id, &path).is_visible() {
                return Err(FilesFault::PathNotFound(path.clone()));
            }
            if !path.is_root() {
                match nodes.get(path.as_str()).filter(|n| n.is_live()) {
                    Some(n) if n.kind == EntryKind::Directory => {}
                    Some(_) => return Err(FilesFault::NotADirectory(path.clone())),
                    None => return Err(FilesFault::PathNotFound(path.clone())),
                }
            }
            Ok(nodes
                .iter()
                .filter(|(_, n)| n.is_live())
                .filter_map(|(k, n)| RootPath::parse(k).ok().map(|p| (p, n)))
                .filter(|(p, _)| p.parent().as_ref() == Some(&path))
                .filter(|(p, _)| actor.at(state, root_id, p).is_visible())
                .map(|(p, n)| n.entry(root_id, p))
                .collect())
        })
        .await
    }

    async fn entry(&self, root_id: RootId, path: RootPath) -> Result<Entry, FilesFault> {
        let path = path.validate()?;
        self.view(|state, actor| {
            let root = state.root(root_id)?;
            if !actor.at(state, root_id, &path).is_visible() {
                return Err(FilesFault::PathNotFound(path));
            }
            if path.is_root() {
                return Ok(root_entry(&root.root));
            }
            root.nodes
                .get(path.as_str())
                .filter(|n| n.is_live())
                .map(|n| n.entry(root_id, path.clone()))
                .ok_or(FilesFault::PathNotFound(path))
        })
        .await
    }

    async fn create_dirs(
        &self,
        root_id: RootId,
        paths: Vec<RootPath>,
    ) -> Result<WriteReceipt, FilesFault> {
        let paths = validate_all(paths)?;
        self.mutate(|state, actor, stamp| {
            for path in &paths {
                actor.need(state, root_id, path, Capability::Write)?;
            }
            let (outcomes, events) = Self::batch(state, root_id, stamp, |b| {
                paths
                    .iter()
                    .map(|path| {
                        b.ensure_dir(path)?;
                        Ok(crate::model::Outcome {
                            path: path.clone(),
                            landed_at: Some(path.clone()),
                            skipped: false,
                        })
                    })
                    .collect::<Result<Vec<_>, FilesFault>>()
            })?;
            Ok((WriteReceipt { root_id, outcomes }, events))
        })
        .await
    }

    async fn rename(
        &self,
        root_id: RootId,
        path: RootPath,
        name: String,
    ) -> Result<WriteReceipt, FilesFault> {
        let path = path.validate()?;
        let to = path.with_name(name.trim())?;
        self.move_paths(
            root_id,
            vec![Relocation { from: path, to }],
            OnConflict::Fail,
        )
        .await
    }

    async fn move_paths(
        &self,
        root_id: RootId,
        moves: Vec<Relocation>,
        on_conflict: OnConflict,
    ) -> Result<WriteReceipt, FilesFault> {
        self.relocate(root_id, moves, on_conflict, false).await
    }

    async fn copy_paths(
        &self,
        root_id: RootId,
        copies: Vec<Relocation>,
        on_conflict: OnConflict,
    ) -> Result<WriteReceipt, FilesFault> {
        self.relocate(root_id, copies, on_conflict, true).await
    }

    async fn delete_paths(
        &self,
        root_id: RootId,
        paths: Vec<RootPath>,
    ) -> Result<WriteReceipt, FilesFault> {
        let paths = validate_all(paths)?;
        self.mutate(|state, actor, stamp| {
            for path in &paths {
                actor.need(state, root_id, path, Capability::Write)?;
            }
            let (outcomes, events) = Self::batch(state, root_id, stamp, |b| {
                paths
                    .iter()
                    .map(|path| {
                        b.remove(path)?;
                        Ok(crate::model::Outcome {
                            path: path.clone(),
                            landed_at: None,
                            skipped: false,
                        })
                    })
                    .collect::<Result<Vec<_>, FilesFault>>()
            })?;
            Ok((WriteReceipt { root_id, outcomes }, events))
        })
        .await
    }

    // ── Bytes ──────────────────────────────────────────────────────

    async fn begin_upload(&self, spec: UploadSpec) -> Result<UploadPlan, FilesFault> {
        let spec = UploadSpec {
            path: spec.path.validate()?,
            ..spec
        };
        if spec.path.is_root() {
            return Err(FilesFault::NotAFile(spec.path));
        }
        if spec.content.as_ref().is_some_and(|c| !c.is_sha256()) {
            return Err(FilesFault::invalid("content is a sha256: 64 hex digits"));
        }
        let held = match &spec.content {
            Some(c) => {
                let hash =
                    Sha256::from_hex(c.as_str()).map_err(|e| FilesFault::invalid(e.to_string()))?;
                self.server.blobs().has(hash).await.map_err(io)?
            }
            None => false,
        };
        let (files, _) = self.ctx().await?;
        let plan = self
            .mutate(|state, actor, _| {
                actor.need(state, spec.root_id, &spec.path, Capability::Write)?;
                let node = state.root(spec.root_id)?.nodes.get(spec.path.as_str());
                if node.is_some_and(|n| n.is_live() && n.kind == EntryKind::Directory) {
                    return Err(FilesFault::NotAFile(spec.path.clone()));
                }
                Self::check_expect(node, &spec.path, spec.expect.as_ref())?;
                let occupant = node.filter(|n| n.is_live());
                if let Some(quota) = state.quota {
                    let used = state
                        .used()
                        .saturating_sub(occupant.map_or(0, Node::live_size));
                    if used.saturating_add(spec.size) > quota {
                        return Err(FilesFault::QuotaExceeded {
                            used: state.used(),
                            quota,
                            adding: spec.size,
                        });
                    }
                }
                let conflict = occupant.map(|n| n.entry(spec.root_id, spec.path.clone()));
                // The org's own content only: a hash another org holds is
                // not proof the caller has the bytes.
                let dedup = held && spec.content.as_ref().is_some_and(|c| state.references(c));
                let upload = Upload {
                    id: UploadId::generate(),
                    spec: spec.clone(),
                    received: Vec::new(),
                    by: actor.label.clone(),
                    staged: !dedup,
                };
                let plan = UploadPlan {
                    upload_id: upload.id,
                    needed: upload.needed(),
                    conflict,
                };
                state.uploads.push(upload);
                Ok((plan, Vec::new()))
            })
            .await?;
        if !plan.needed.is_empty() || spec.size == 0 {
            let staged = files.staging(plan.upload_id);
            if let Some(dir) = staged.parent() {
                tokio::fs::create_dir_all(dir).await.map_err(io)?;
            }
            tokio::fs::File::create(&staged).await.map_err(io)?;
        }
        Ok(plan)
    }

    async fn upload_progress(&self, upload_id: UploadId) -> Result<UploadProgress, FilesFault> {
        self.view(|state, actor| {
            let upload = Self::upload(state, actor, upload_id)?;
            Ok(UploadProgress {
                upload_id,
                root_id: upload.spec.root_id,
                path: upload.spec.path.clone(),
                received: upload.received_bytes(),
                total: upload.spec.size,
                needed: upload.needed(),
            })
        })
        .await
    }

    async fn send_bytes(
        &self,
        upload_id: UploadId,
        mut frames: Rx<UploadFrame>,
    ) -> Result<Received, FilesFault> {
        let (files, _) = self.ctx().await?;
        let upload = self
            .view(|state, actor| Self::upload(state, actor, upload_id).cloned())
            .await?;
        if !upload.staged {
            return Ok(Received {
                upload_id,
                written: 0,
                needed: Vec::new(),
            });
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(files.staging(upload_id))
            .await
            .map_err(io)?;
        let mut got: Vec<ByteRange> = Vec::new();
        let mut written = 0u64;
        let outcome = loop {
            let frame = match frames.recv().await {
                Ok(Some(frame)) => frame,
                Ok(None) => break Ok(()),
                Err(e) => break Err(FilesFault::Io(format!("the upload stream broke: {e:?}"))),
            };
            let mut owned = None;
            let _ = frame.map(|f| owned = Some(f));
            let Some(UploadFrame::Chunk { offset, bytes }) = owned else {
                break Ok(());
            };
            let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            let end = offset.saturating_add(len);
            if end > upload.spec.size {
                break Err(FilesFault::invalid(format!(
                    "bytes {offset}..{end} lie past the file's {} bytes",
                    upload.spec.size
                )));
            }
            if let Err(e) = write_at(&mut file, offset, &bytes).await {
                break Err(e);
            }
            add_range(&mut got, ByteRange { start: offset, end });
            written = written.saturating_add(len);
        };
        file.sync_data().await.map_err(io)?;
        // What arrived counts even when the stream broke: the client
        // resumes from the server's account of it.
        let needed = self
            .mutate(|state, actor, _| {
                let label = actor.label.clone();
                let upload = state
                    .uploads
                    .iter_mut()
                    .find(|u| u.id == upload_id && u.by == label)
                    .ok_or(FilesFault::UploadNotFound(upload_id))?;
                for range in &got {
                    add_range(&mut upload.received, *range);
                }
                Ok((upload.needed(), Vec::new()))
            })
            .await?;
        outcome?;
        Ok(Received {
            upload_id,
            written,
            needed,
        })
    }

    async fn complete_upload(
        &self,
        upload_id: UploadId,
        on_conflict: OnConflict,
    ) -> Result<Entry, FilesFault> {
        let (files, _) = self.ctx().await?;
        let upload = self
            .view(|state, actor| Self::upload(state, actor, upload_id).cloned())
            .await?;
        if !upload.needed().is_empty() {
            return Err(FilesFault::Incomplete(upload_id));
        }
        let content = match self.store_staged(&files, &upload).await {
            Ok(content) => content,
            Err(e @ FilesFault::Invalid(_)) => {
                // Bytes that are not what the client named: this upload
                // cannot land; it must begin again.
                let _ = self.abort_upload(upload_id).await;
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        let spec = upload.spec.clone();
        let landed = self
            .mutate(|state, actor, stamp| {
                actor.need(state, spec.root_id, &spec.path, Capability::Write)?;
                state.uploads.retain(|u| u.id != upload_id);
                let node = state.root(spec.root_id)?.nodes.get(spec.path.as_str());
                Self::check_expect(node, &spec.path, spec.expect.as_ref())?;
                let mut target = spec.path.clone();
                match node.filter(|n| n.is_live()) {
                    Some(n) if n.kind == EntryKind::Directory => {
                        return Err(FilesFault::NotAFile(spec.path.clone()));
                    }
                    Some(n) => match on_conflict {
                        OnConflict::Fail => {
                            return Err(FilesFault::Exists {
                                path: spec.path.clone(),
                            });
                        }
                        OnConflict::KeepExisting => {
                            return Ok((n.entry(spec.root_id, spec.path.clone()), Vec::new()));
                        }
                        OnConflict::Replace => {}
                        OnConflict::KeepBoth => {
                            target = Self::batch(state, spec.root_id, stamp, |b| {
                                Ok(b.free_name(&spec.path))
                            })?
                            .0;
                        }
                    },
                    None => {}
                }
                if target != spec.path {
                    actor.need(state, spec.root_id, &target, Capability::Write)?;
                }
                let ((), events) = Self::batch(state, spec.root_id, stamp, |b| {
                    b.write_file(&target, content.clone(), spec.size)
                })?;
                let entry = state
                    .root(spec.root_id)?
                    .nodes
                    .get(target.as_str())
                    .map(|n| n.entry(spec.root_id, target.clone()))
                    .ok_or_else(|| FilesFault::Internal("a landed file is missing".into()))?;
                Ok((entry, events))
            })
            .await;
        if landed.is_ok() {
            let _ = tokio::fs::remove_file(files.staging(upload_id)).await;
        }
        landed
    }

    async fn abort_upload(&self, upload_id: UploadId) -> Result<(), FilesFault> {
        let (files, _) = self.ctx().await?;
        let gone = self
            .mutate(|state, actor, _| {
                let upload = Self::upload(state, actor, upload_id)?.clone();
                state.uploads.retain(|u| u.id != upload_id);
                Ok((vec![upload], Vec::new()))
            })
            .await?;
        discard(&files, &gone).await;
        Ok(())
    }

    async fn read(&self, request: ReadRequest, out: Tx<Vec<u8>>) -> Result<ReadDone, FilesFault> {
        let path = request.path.validate()?;
        let version = self
            .view(|state, actor| {
                actor.need(state, request.root_id, &path, Capability::Read)?;
                if request.version.is_some() {
                    actor.need(state, request.root_id, &path, Capability::History)?;
                }
                let node = state
                    .root(request.root_id)?
                    .nodes
                    .get(path.as_str())
                    .filter(|n| n.is_file())
                    .ok_or_else(|| FilesFault::PathNotFound(path.clone()))?;
                request.version.map_or_else(
                    || {
                        node.current()
                            .filter(|v| v.content.is_some())
                            .cloned()
                            .ok_or_else(|| FilesFault::PathNotFound(path.clone()))
                    },
                    |id| {
                        node.versions
                            .iter()
                            .find(|v| v.id == id)
                            .cloned()
                            .ok_or(FilesFault::VersionNotFound(id))
                    },
                )
            })
            .await?;
        let content = version
            .content
            .ok_or_else(|| FilesFault::invalid("that version is a delete: it has no bytes"))?;
        let total = version.size;
        let range = request.range.map_or(
            ByteRange {
                start: 0,
                end: total,
            },
            |r| {
                let end = r.end.min(total);
                ByteRange {
                    start: r.start.min(end),
                    end,
                }
            },
        );
        let hash =
            Sha256::from_hex(content.as_str()).map_err(|e| FilesFault::Internal(e.to_string()))?;
        let mut file = tokio::fs::File::open(self.server.blobs().path_for(&hash))
            .await
            .map_err(io)?;
        file.seek(std::io::SeekFrom::Start(range.start))
            .await
            .map_err(io)?;
        let mut left = range.len();
        let mut buf = vec![0u8; FRAME];
        while left > 0 {
            let want = usize::try_from(left).unwrap_or(FRAME).min(FRAME);
            let chunk = buf.get_mut(..want).unwrap_or_default();
            file.read_exact(chunk).await.map_err(io)?;
            if out.send(chunk.to_vec()).await.is_err() {
                return Err(FilesFault::Io("the reader went away".into()));
            }
            left = left.saturating_sub(u64::try_from(want).unwrap_or(left));
        }
        Ok(ReadDone {
            content,
            total,
            range,
        })
    }

    // ── Versions ───────────────────────────────────────────────────

    async fn history(&self, root_id: RootId, path: RootPath) -> Result<Vec<Version>, FilesFault> {
        let path = path.validate()?;
        self.view(|state, actor| {
            actor.need(state, root_id, &path, Capability::History)?;
            state
                .root(root_id)?
                .nodes
                .get(path.as_str())
                .filter(|n| n.is_file())
                .map(|n| n.versions.clone())
                .ok_or(FilesFault::PathNotFound(path))
        })
        .await
    }

    async fn restore(
        &self,
        root_id: RootId,
        path: RootPath,
        version: VersionId,
    ) -> Result<Entry, FilesFault> {
        let path = path.validate()?;
        self.mutate(|state, actor, stamp| {
            actor.need(state, root_id, &path, Capability::History)?;
            actor.need(state, root_id, &path, Capability::Write)?;
            let node = state
                .root(root_id)?
                .nodes
                .get(path.as_str())
                .filter(|n| n.is_file())
                .ok_or_else(|| FilesFault::PathNotFound(path.clone()))?;
            let old = node
                .versions
                .iter()
                .find(|v| v.id == version)
                .ok_or(FilesFault::VersionNotFound(version))?;
            let content = old.content.clone().ok_or_else(|| {
                FilesFault::invalid("that version is a delete: restore one before it")
            })?;
            let size = old.size;
            let ((), events) = Self::batch(state, root_id, stamp, |b| {
                b.write_file(&path, content, size)
            })?;
            let entry = state
                .root(root_id)?
                .nodes
                .get(path.as_str())
                .map(|n| n.entry(root_id, path.clone()))
                .ok_or_else(|| FilesFault::Internal("a restored file is missing".into()))?;
            Ok((entry, events))
        })
        .await
    }

    async fn label(
        &self,
        root_id: RootId,
        path: RootPath,
        version: VersionId,
        name: String,
    ) -> Result<VersionLabel, FilesFault> {
        let path = path.validate()?;
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(FilesFault::invalid("a label needs a name"));
        }
        self.mutate(|state, actor, stamp| {
            actor.need(state, root_id, &path, Capability::Write)?;
            let held = state
                .root(root_id)?
                .nodes
                .get(path.as_str())
                .is_some_and(|n| n.versions.iter().any(|v| v.id == version));
            if !held {
                return Err(FilesFault::VersionNotFound(version));
            }
            let same = |l: &&VersionLabel| l.root_id == root_id && l.path == path && l.name == name;
            if let Some(existing) = state.labels.iter().find(same) {
                return if existing.version == version {
                    Ok((existing.clone(), Vec::new()))
                } else {
                    Err(FilesFault::Exists { path: path.clone() })
                };
            }
            let label = VersionLabel {
                id: LabelId::generate(),
                root_id,
                path: path.clone(),
                version,
                name,
                by: stamp.by.to_owned(),
                at_us: stamp.now,
            };
            state.labels.push(label.clone());
            Ok((label.clone(), vec![FilesEvent::Labelled(label)]))
        })
        .await
    }

    async fn unlabel(&self, label_id: LabelId) -> Result<VersionLabel, FilesFault> {
        self.mutate(|state, actor, _| {
            let label = state
                .labels
                .iter()
                .find(|l| l.id == label_id)
                .cloned()
                .ok_or(FilesFault::LabelNotFound(label_id))?;
            actor
                .need(state, label.root_id, &label.path, Capability::Write)
                .map_err(|e| match e {
                    FilesFault::PathNotFound(_) => FilesFault::LabelNotFound(label_id),
                    other => other,
                })?;
            state.labels.retain(|l| l.id != label_id);
            Ok((label.clone(), vec![FilesEvent::Unlabelled(label)]))
        })
        .await
    }

    async fn labels(
        &self,
        root_id: RootId,
        path: Option<RootPath>,
    ) -> Result<Vec<VersionLabel>, FilesFault> {
        let path = path.map(|p| p.validate()).transpose()?;
        self.view(|state, actor| {
            if let Some(path) = &path {
                actor.need(state, root_id, path, Capability::History)?;
            } else if !actor.sees_root(state, state.root(root_id)?.root.id, now_us()) {
                return Err(FilesFault::RootNotFound(root_id));
            }
            Ok(state
                .labels
                .iter()
                .filter(|l| l.root_id == root_id)
                .filter(|l| path.as_ref().is_none_or(|p| &l.path == p))
                .filter(|l| {
                    actor
                        .at(state, root_id, &l.path)
                        .allows(Capability::History)
                })
                .cloned()
                .collect())
        })
        .await
    }

    // ── Access ─────────────────────────────────────────────────────

    async fn grant(
        &self,
        root_id: RootId,
        path: RootPath,
        user: String,
        capabilities: Vec<Capability>,
    ) -> Result<Grant, FilesFault> {
        let path = path.validate()?;
        let user = user.trim().to_owned();
        if user.is_empty() || capabilities.is_empty() {
            return Err(FilesFault::invalid(
                "a grant is for someone and conveys something",
            ));
        }
        self.mutate(|state, actor, stamp| {
            actor.need(state, root_id, &path, Capability::Share)?;
            // No more than the granter holds here: `Share` passes on, it
            // never escalates.
            let held = actor.at(state, root_id, &path);
            if let Some(over) = capabilities.iter().find(|c| !held.allows(**c)) {
                return Err(FilesFault::denied(format!("{over:?}"), path.clone()));
            }
            let grant = Grant {
                id: GrantId::generate(),
                root_id,
                path: path.clone(),
                user,
                capabilities: canonical(capabilities),
                granted_by: stamp.by.to_owned(),
                granted_us: stamp.now,
                expires_us: None,
            };
            state.grants.push(grant.clone());
            Ok((grant.clone(), vec![FilesEvent::Granted(grant)]))
        })
        .await
    }

    async fn revoke(&self, grant_id: GrantId) -> Result<Grant, FilesFault> {
        self.mutate(|state, actor, _| {
            let grant = state
                .grants
                .iter()
                .find(|g| g.id == grant_id)
                .cloned()
                .ok_or(FilesFault::GrantNotFound(grant_id))?;
            actor
                .need(state, grant.root_id, &grant.path, Capability::Share)
                .map_err(|e| match e {
                    FilesFault::PathNotFound(_) => FilesFault::GrantNotFound(grant_id),
                    other => other,
                })?;
            state.grants.retain(|g| g.id != grant_id);
            Ok((grant.clone(), vec![FilesEvent::Revoked(grant)]))
        })
        .await
    }

    async fn grants(
        &self,
        root_id: RootId,
        path: Option<RootPath>,
    ) -> Result<Vec<Grant>, FilesFault> {
        let path = path.map(|p| p.validate()).transpose()?;
        self.view(|state, actor| {
            let at = path.clone().unwrap_or_default();
            actor.need(state, root_id, &at, Capability::Share)?;
            let now = now_us();
            Ok(state
                .grants
                .iter()
                .filter(|g| g.root_id == root_id && g.is_live(now))
                .filter(|g| path.as_ref().is_none_or(|p| p.is_within(&g.path)))
                .cloned()
                .collect())
        })
        .await
    }

    async fn effective(&self, root_id: RootId, path: RootPath) -> Result<Effective, FilesFault> {
        let path = path.validate()?;
        self.view(|state, actor| {
            state.root(root_id)?;
            match actor.at(state, root_id, &path) {
                Access::Absent => Err(FilesFault::PathNotFound(path)),
                Access::Holds(capabilities) => Ok(Effective {
                    root_id,
                    path,
                    capabilities,
                }),
            }
        })
        .await
    }
}

impl Acting {
    async fn relocate(
        &self,
        root_id: RootId,
        moves: Vec<Relocation>,
        on_conflict: OnConflict,
        copy: bool,
    ) -> Result<WriteReceipt, FilesFault> {
        let moves = moves
            .into_iter()
            .map(|m| {
                Ok(Relocation {
                    from: m.from.validate()?,
                    to: m.to.validate()?,
                })
            })
            .collect::<Result<Vec<_>, FilesFault>>()?;
        self.mutate(|state, actor, stamp| {
            for m in &moves {
                let source = if copy {
                    Capability::Read
                } else {
                    Capability::Write
                };
                actor.need(state, root_id, &m.from, source)?;
                actor.need(state, root_id, &m.to, Capability::Write)?;
            }
            let (outcomes, events) = Self::batch(state, root_id, stamp, |b| {
                moves
                    .iter()
                    .map(|m| b.relocate(&m.from, &m.to, on_conflict, copy))
                    .collect::<Result<Vec<_>, FilesFault>>()
            })?;
            // A name derived under `KeepBoth` is checked like the one asked.
            for landed in outcomes.iter().filter_map(|o| o.landed_at.as_ref()) {
                actor.need(state, root_id, landed, Capability::Write)?;
            }
            Ok((WriteReceipt { root_id, outcomes }, events))
        })
        .await
    }
}

/// Refuse unless `actor` keeps the org's roots and `root` exists.
fn steward(state: &State, actor: &Actor, root: RootId) -> Result<(), FilesFault> {
    state.root(root)?;
    if actor.steward {
        Ok(())
    } else if actor.sees_root(state, root, now_us()) {
        Err(FilesFault::denied("steward", RootPath::root()))
    } else {
        Err(FilesFault::RootNotFound(root))
    }
}

fn validate_all(paths: Vec<RootPath>) -> Result<Vec<RootPath>, FilesFault> {
    paths
        .into_iter()
        .map(|p| p.validate().map_err(FilesFault::from))
        .collect()
}

async fn write_at(file: &mut tokio::fs::File, offset: u64, bytes: &[u8]) -> Result<(), FilesFault> {
    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(io)?;
    file.write_all(bytes).await.map_err(io)
}

/// Remove dropped uploads' staged bytes.
async fn discard(files: &OrgFiles, uploads: &[Upload]) {
    for upload in uploads {
        let _ = tokio::fs::remove_file(files.staging(upload.id)).await;
    }
}

// ── The server, for the call running now ───────────────────────────

/// Every method for the gate's principal in the lane's org.
impl FilesService for FilesServer {
    async fn ensure_root(&self, dir: String, name: String) -> Result<Root, FilesFault> {
        self.for_call()?.ensure_root(dir, name).await
    }
    async fn roots(&self) -> Result<Vec<Root>, FilesFault> {
        self.for_call()?.roots().await
    }
    async fn root(&self, root_id: RootId) -> Result<Root, FilesFault> {
        self.for_call()?.root(root_id).await
    }
    async fn rename_root(&self, root_id: RootId, name: String) -> Result<Root, FilesFault> {
        self.for_call()?.rename_root(root_id, name).await
    }
    async fn release_root(&self, root_id: RootId) -> Result<(), FilesFault> {
        self.for_call()?.release_root(root_id).await
    }
    async fn usage(&self) -> Result<Usage, FilesFault> {
        self.for_call()?.usage().await
    }
    async fn browse(&self, root_id: RootId, path: RootPath) -> Result<Vec<Entry>, FilesFault> {
        self.for_call()?.browse(root_id, path).await
    }
    async fn entry(&self, root_id: RootId, path: RootPath) -> Result<Entry, FilesFault> {
        self.for_call()?.entry(root_id, path).await
    }
    async fn create_dirs(
        &self,
        root_id: RootId,
        paths: Vec<RootPath>,
    ) -> Result<WriteReceipt, FilesFault> {
        self.for_call()?.create_dirs(root_id, paths).await
    }
    async fn rename(
        &self,
        root_id: RootId,
        path: RootPath,
        name: String,
    ) -> Result<WriteReceipt, FilesFault> {
        self.for_call()?.rename(root_id, path, name).await
    }
    async fn move_paths(
        &self,
        root_id: RootId,
        moves: Vec<Relocation>,
        on_conflict: OnConflict,
    ) -> Result<WriteReceipt, FilesFault> {
        self.for_call()?
            .move_paths(root_id, moves, on_conflict)
            .await
    }
    async fn copy_paths(
        &self,
        root_id: RootId,
        copies: Vec<Relocation>,
        on_conflict: OnConflict,
    ) -> Result<WriteReceipt, FilesFault> {
        self.for_call()?
            .copy_paths(root_id, copies, on_conflict)
            .await
    }
    async fn delete_paths(
        &self,
        root_id: RootId,
        paths: Vec<RootPath>,
    ) -> Result<WriteReceipt, FilesFault> {
        self.for_call()?.delete_paths(root_id, paths).await
    }
    async fn begin_upload(&self, spec: UploadSpec) -> Result<UploadPlan, FilesFault> {
        self.for_call()?.begin_upload(spec).await
    }
    async fn upload_progress(&self, upload_id: UploadId) -> Result<UploadProgress, FilesFault> {
        self.for_call()?.upload_progress(upload_id).await
    }
    async fn send_bytes(
        &self,
        upload_id: UploadId,
        frames: Rx<UploadFrame>,
    ) -> Result<Received, FilesFault> {
        self.for_call()?.send_bytes(upload_id, frames).await
    }
    async fn complete_upload(
        &self,
        upload_id: UploadId,
        on_conflict: OnConflict,
    ) -> Result<Entry, FilesFault> {
        self.for_call()?
            .complete_upload(upload_id, on_conflict)
            .await
    }
    async fn abort_upload(&self, upload_id: UploadId) -> Result<(), FilesFault> {
        self.for_call()?.abort_upload(upload_id).await
    }
    async fn read(&self, request: ReadRequest, out: Tx<Vec<u8>>) -> Result<ReadDone, FilesFault> {
        FilesService::read(&self.for_call()?, request, out).await
    }
    async fn history(&self, root_id: RootId, path: RootPath) -> Result<Vec<Version>, FilesFault> {
        self.for_call()?.history(root_id, path).await
    }
    async fn restore(
        &self,
        root_id: RootId,
        path: RootPath,
        version: VersionId,
    ) -> Result<Entry, FilesFault> {
        self.for_call()?.restore(root_id, path, version).await
    }
    async fn label(
        &self,
        root_id: RootId,
        path: RootPath,
        version: VersionId,
        name: String,
    ) -> Result<VersionLabel, FilesFault> {
        self.for_call()?.label(root_id, path, version, name).await
    }
    async fn unlabel(&self, label_id: LabelId) -> Result<VersionLabel, FilesFault> {
        self.for_call()?.unlabel(label_id).await
    }
    async fn labels(
        &self,
        root_id: RootId,
        path: Option<RootPath>,
    ) -> Result<Vec<VersionLabel>, FilesFault> {
        self.for_call()?.labels(root_id, path).await
    }
    async fn grant(
        &self,
        root_id: RootId,
        path: RootPath,
        user: String,
        capabilities: Vec<Capability>,
    ) -> Result<Grant, FilesFault> {
        self.for_call()?
            .grant(root_id, path, user, capabilities)
            .await
    }
    async fn revoke(&self, grant_id: GrantId) -> Result<Grant, FilesFault> {
        self.for_call()?.revoke(grant_id).await
    }
    async fn grants(
        &self,
        root_id: RootId,
        path: Option<RootPath>,
    ) -> Result<Vec<Grant>, FilesFault> {
        self.for_call()?.grants(root_id, path).await
    }
    async fn effective(&self, root_id: RootId, path: RootPath) -> Result<Effective, FilesFault> {
        self.for_call()?.effective(root_id, path).await
    }
}

/// The events stream, per subscriber: what it may see, as it may see it
/// when the event happens (a revoked grant stops the next event).
impl FilesServiceStreamSource for FilesServer {
    fn events_attach(&self, root_id: Option<RootId>, sink: EventSink<FilesEvent>) {
        // No org or no principal: the sink drops and the stream ends.
        let Ok(acting) = self.for_call() else {
            return;
        };
        let server = self.clone();
        tokio::spawn(async move {
            let Ok(files) = server.org(&acting.org).await else {
                return;
            };
            let events = files.hub.subscribe(super::EVENT_BACKLOG);
            if sink.send(FilesEvent::Attached).await.is_err() {
                return;
            }
            while let Ok(event) = events.recv().await {
                // Asked per event: a role or a grant may have changed.
                let Ok(actor) = server.actor(&acting.org, &acting.who) else {
                    return;
                };
                let visible = {
                    let state = files.state.lock().await;
                    Self::visible(&actor, &state, root_id, event)
                };
                if let Some(event) = visible
                    && sink.send(event).await.is_err()
                {
                    return;
                }
            }
        });
    }
}
