//! Renditions on a server: what a [`Transcoder`] makes of a version's
//! bytes, off the request path, kept beside the blobs.
//!
//! Every version that lands schedules its source's renditions (a save,
//! a copy, a restore: `FilesServer::with_renditions`); a request for a
//! set or for one rendition schedules whatever is missing. A build runs
//! on its own task, a few at a time ([`Renditions::concurrency`]): it
//! asks the transcoder what the source is, then makes each kind the
//! class yields that is not already made, and puts each in the blob
//! store the moment it is made.
//!
//! What is made is recorded per source under
//! `<data>/renditions/<hex[0..2]>/<hex>.json`, written atomically after
//! every rendition, so a restart keeps it and a build cut short resumes
//! where it stopped. A record from another [`RECIPE`], or one whose
//! rendition blob is gone (collected, a lost disk), counts as missing,
//! and is made again. A failure is remembered until the server restarts,
//! so a transcoder that cannot make a kind is not asked again on every
//! request, and one fixed by a restart (an `ffmpeg` installed) is.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use architect::blob::{BlobStore as _, LocalFs, Sha256};
use facet::Facet;
use futures_util::future::BoxFuture;
use tokio::sync::Semaphore;

use super::io;
use crate::error::FilesFault;
use crate::id::ContentId;
use crate::rendition::{
    MediaClass, RECIPE, RenditionKind, RenditionSet, RenditionState, RenditionStatus,
};

/// Why a transcoder could not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TranscodeError(pub String);

impl TranscodeError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self(reason.into())
    }
}

/// What makes renditions: `ffmpeg` on a server (`FfmpegTranscoder`,
/// feature `ffmpeg`), a cloud encoder, a fake in a test.
///
/// Both calls get the source as a file on disk (the blob itself, never a
/// copy) and run off the request path, a few at a time.
pub trait Transcoder: Send + Sync + 'static {
    /// What the source is: whether it has a picture, sound, or neither.
    fn probe<'a>(&'a self, source: &'a Path) -> BoxFuture<'a, Result<MediaClass, TranscodeError>>;

    /// Write the `kind` rendition of `source` to `out` (a fresh path whose
    /// extension says the format: `.mp4`, `.jpg`, `.json`, `.m4a`).
    fn render<'a>(
        &'a self,
        kind: RenditionKind,
        source: &'a Path,
        out: &'a Path,
    ) -> BoxFuture<'a, Result<(), TranscodeError>>;
}

/// What one source's record on disk says.
#[derive(Debug, Clone, Default, Facet)]
struct Record {
    recipe: u32,
    class: Option<MediaClass>,
    made: Vec<Made>,
}

/// One rendition, made.
#[derive(Debug, Clone, Facet)]
struct Made {
    kind: RenditionKind,
    content: ContentId,
    size: u64,
}

/// What a source has failed at, this run: the probe (`None`) or a kind.
type Failures = HashMap<(ContentId, Option<RenditionKind>), String>;

/// A server's renditions.
#[derive(Clone)]
pub struct Renditions {
    inner: Arc<Engine>,
}

struct Engine {
    dir: PathBuf,
    blobs: LocalFs,
    transcoder: Arc<dyn Transcoder>,
    permits: Semaphore,
    building: Mutex<HashSet<ContentId>>,
    failed: Mutex<Failures>,
}

impl Renditions {
    /// Renditions recorded under `dir`, their bytes in `blobs` (the same
    /// store the sources are in), made by `transcoder` one at a time.
    pub fn new(dir: impl Into<PathBuf>, blobs: LocalFs, transcoder: impl Transcoder) -> Self {
        Self::with_transcoder(dir, blobs, Arc::new(transcoder), 1)
    }

    fn with_transcoder(
        dir: impl Into<PathBuf>,
        blobs: LocalFs,
        transcoder: Arc<dyn Transcoder>,
        concurrency: usize,
    ) -> Self {
        Self {
            inner: Arc::new(Engine {
                dir: dir.into(),
                blobs,
                transcoder,
                permits: Semaphore::new(concurrency.max(1)),
                building: Mutex::new(HashSet::new()),
                failed: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The same renditions, `n` builds at a time.
    #[must_use]
    pub fn concurrency(self, n: usize) -> Self {
        let inner = &self.inner;
        Self::with_transcoder(
            inner.dir.clone(),
            inner.blobs.clone(),
            Arc::clone(&inner.transcoder),
            n,
        )
    }

    /// `content`'s renditions as they are, scheduling a build when any is
    /// missing.
    pub async fn set(&self, content: &ContentId) -> Result<RenditionSet, FilesFault> {
        let record = self.record(content).await?;
        let set = self.view(content, record.as_ref()).await?;
        let wanting = (set.class.is_none() && set.problem.is_none())
            || set
                .kinds
                .iter()
                .any(|k| matches!(k.state, RenditionState::Pending));
        if wanting {
            self.schedule(content.clone());
        }
        Ok(set)
    }

    /// The `kind` rendition of `content`: its bytes and length, or why
    /// there is none (yet).
    pub async fn ready(
        &self,
        content: &ContentId,
        kind: RenditionKind,
    ) -> Result<(Sha256, u64), FilesFault> {
        let set = self.set(content).await?;
        if let Some(reason) = set.problem {
            return Err(FilesFault::NoRendition { kind, reason });
        }
        let Some(class) = set.class else {
            return Err(FilesFault::RenditionPending(kind));
        };
        match set.state(kind) {
            Some(RenditionState::Ready { content, size }) => Ok((hash_of(content)?, *size)),
            Some(RenditionState::Pending) => Err(FilesFault::RenditionPending(kind)),
            Some(RenditionState::Failed { reason }) => Err(FilesFault::NoRendition {
                kind,
                reason: reason.clone(),
            }),
            None => Err(FilesFault::NoRendition {
                kind,
                reason: format!("it does not apply to {class:?} sources"),
            }),
        }
    }

    /// Make `content`'s missing renditions, on a task of their own; a
    /// source already being built is left to that build.
    pub fn schedule(&self, content: ContentId) {
        if !self.lock_building().insert(content.clone()) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            let _ = this.build(&content).await;
            this.lock_building().remove(&content);
        });
    }

    /// Make `content`'s missing renditions now, on this task: what
    /// [`Self::schedule`] runs. A kind the transcoder fails at is
    /// remembered as failed; a source the blob store does not hold makes
    /// nothing.
    pub async fn build(&self, content: &ContentId) -> Result<(), FilesFault> {
        let _permit = self
            .inner
            .permits
            .acquire()
            .await
            .map_err(|e| FilesFault::Internal(e.to_string()))?;
        let source = self.inner.blobs.path_for(&hash_of(content)?);
        if !tokio::fs::try_exists(&source).await.map_err(io)? {
            return Ok(());
        }
        let mut record = self.record(content).await?.unwrap_or_else(|| Record {
            recipe: RECIPE,
            ..Record::default()
        });
        let class = if let Some(class) = record.class {
            class
        } else {
            match self.inner.transcoder.probe(&source).await {
                Ok(class) => {
                    record.class = Some(class);
                    self.save(content, &record).await?;
                    class
                }
                Err(e) => {
                    self.fail(content, None, e.0);
                    return Ok(());
                }
            }
        };
        for &kind in RenditionKind::ladder(class) {
            if self.kind_failed(content, kind).is_some() || self.has(&record, kind).await? {
                continue;
            }
            match self.make(kind, &source).await? {
                Ok(made) => {
                    record.made.retain(|m| m.kind != kind);
                    record.made.push(made);
                    self.save(content, &record).await?;
                }
                Err(e) => self.fail(content, Some(kind), e.0),
            }
        }
        Ok(())
    }

    /// Render one kind into the blob store. The outer error is ours (the
    /// disk), the inner the transcoder's.
    async fn make(
        &self,
        kind: RenditionKind,
        source: &Path,
    ) -> Result<Result<Made, TranscodeError>, FilesFault> {
        let work = self.inner.dir.join(".work");
        tokio::fs::create_dir_all(&work).await.map_err(io)?;
        let out = work.join(format!("{}.{}", uuid::Uuid::new_v4(), extension(kind)));
        let rendered = self.inner.transcoder.render(kind, source, &out).await;
        let stored = match rendered {
            Ok(()) => self.store(&out).await.map(|(hash, size)| {
                Ok(Made {
                    kind,
                    content: ContentId::new(hash.to_hex()),
                    size,
                })
            }),
            Err(e) => Ok(Err(e)),
        };
        let _ = tokio::fs::remove_file(&out).await;
        stored
    }

    async fn store(&self, out: &Path) -> Result<(Sha256, u64), FilesFault> {
        let file = match tokio::fs::File::open(out).await {
            Ok(file) => file,
            Err(e) => return Err(FilesFault::Io(format!("the transcoder wrote nothing: {e}"))),
        };
        let size = file.metadata().await.map_err(io)?.len();
        let hash = self
            .inner
            .blobs
            .put(None, Box::pin(file))
            .await
            .map_err(io)?;
        Ok((hash, size))
    }

    /// The set as `record` (and this run's failures, and the blob store)
    /// say it is.
    async fn view(
        &self,
        content: &ContentId,
        record: Option<&Record>,
    ) -> Result<RenditionSet, FilesFault> {
        let class = record.and_then(|r| r.class);
        let mut kinds = Vec::new();
        for &kind in class.map_or(&[][..], RenditionKind::ladder) {
            let made = record.and_then(|r| r.made.iter().find(|m| m.kind == kind));
            let state = match (made, self.kind_failed(content, kind)) {
                (Some(m), _) if self.held(&m.content).await? => RenditionState::Ready {
                    content: m.content.clone(),
                    size: m.size,
                },
                (_, Some(reason)) => RenditionState::Failed { reason },
                _ => RenditionState::Pending,
            };
            kinds.push(RenditionStatus { kind, state });
        }
        Ok(RenditionSet {
            content: content.clone(),
            class,
            kinds,
            problem: class
                .is_none()
                .then(|| self.probe_failed(content))
                .flatten(),
        })
    }

    async fn has(&self, record: &Record, kind: RenditionKind) -> Result<bool, FilesFault> {
        match record.made.iter().find(|m| m.kind == kind) {
            Some(m) => self.held(&m.content).await,
            None => Ok(false),
        }
    }

    async fn held(&self, content: &ContentId) -> Result<bool, FilesFault> {
        self.inner.blobs.has(hash_of(content)?).await.map_err(io)
    }

    fn path(&self, content: &ContentId) -> PathBuf {
        let hex = content.as_str();
        self.inner
            .dir
            .join(hex.get(..2).unwrap_or("00"))
            .join(format!("{hex}.json"))
    }

    /// `content`'s record, if it has one by this recipe.
    async fn record(&self, content: &ContentId) -> Result<Option<Record>, FilesFault> {
        match tokio::fs::read_to_string(self.path(content)).await {
            Ok(json) => Ok(facet_json::from_str::<Record>(&json)
                .ok()
                .filter(|r| r.recipe == RECIPE)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(e)),
        }
    }

    async fn save(&self, content: &ContentId, record: &Record) -> Result<(), FilesFault> {
        let path = self.path(content);
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(io)?;
        }
        let json =
            facet_json::to_string(record).map_err(|e| FilesFault::Internal(e.to_string()))?;
        let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, json).await.map_err(io)?;
        tokio::fs::rename(&tmp, &path).await.map_err(io)
    }

    fn fail(&self, content: &ContentId, kind: Option<RenditionKind>, reason: String) {
        self.lock_failed().insert((content.clone(), kind), reason);
    }

    fn probe_failed(&self, content: &ContentId) -> Option<String> {
        self.lock_failed().get(&(content.clone(), None)).cloned()
    }

    fn kind_failed(&self, content: &ContentId, kind: RenditionKind) -> Option<String> {
        self.lock_failed()
            .get(&(content.clone(), Some(kind)))
            .cloned()
    }

    fn lock_building(&self) -> std::sync::MutexGuard<'_, HashSet<ContentId>> {
        self.inner
            .building
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_failed(&self) -> std::sync::MutexGuard<'_, Failures> {
        self.inner
            .failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

const fn extension(kind: RenditionKind) -> &'static str {
    match kind {
        RenditionKind::Proxy => "mp4",
        RenditionKind::Poster => "jpg",
        RenditionKind::Peaks => "json",
        RenditionKind::Audio => "m4a",
    }
}

fn hash_of(content: &ContentId) -> Result<Sha256, FilesFault> {
    Sha256::from_hex(content.as_str()).map_err(|e| FilesFault::Internal(e.to_string()))
}
