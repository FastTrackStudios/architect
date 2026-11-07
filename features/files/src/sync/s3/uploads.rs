//! Multipart uploads in progress: each part a file under
//! `<data>/s3-uploads/<upload id>/`, joined into one stream when the
//! upload completes, so a large WAV is saved as one version without
//! being held in memory.

use std::path::PathBuf;

use facet::Facet;
use md5::Digest as _;
use s3s::{S3Result, s3_error};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use uuid::Uuid;

/// Who started an upload, and where it lands.
#[derive(Debug, Clone, Facet)]
struct Started {
    bucket: String,
    key: String,
    access_key: String,
}

/// The uploads under one directory. Cheap to clone.
#[derive(Clone)]
pub struct Uploads {
    dir: PathBuf,
}

impl Uploads {
    pub const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// An upload's folder, refusing an id that is not one of ours.
    fn at(&self, id: &str) -> S3Result<PathBuf> {
        Uuid::parse_str(id).map_err(|_| s3_error!(NoSuchUpload))?;
        Ok(self.dir.join(id))
    }

    pub async fn create(
        &self,
        bucket: &str,
        key: &str,
        access_key: &str,
    ) -> std::io::Result<String> {
        let id = Uuid::new_v4().to_string();
        let dir = self.dir.join(&id);
        tokio::fs::create_dir_all(&dir).await?;
        let started = Started {
            bucket: bucket.to_owned(),
            key: key.to_owned(),
            access_key: access_key.to_owned(),
        };
        let json = facet_json::to_string(&started).map_err(std::io::Error::other)?;
        tokio::fs::write(dir.join("upload.json"), json).await?;
        Ok(id)
    }

    /// The upload exists, for this key, started by this caller.
    pub async fn check(&self, id: &str, bucket: &str, key: &str, access_key: &str) -> S3Result<()> {
        let json = tokio::fs::read_to_string(self.at(id)?.join("upload.json"))
            .await
            .map_err(|_| s3_error!(NoSuchUpload))?;
        let started: Started = facet_json::from_str(&json).map_err(|_| s3_error!(NoSuchUpload))?;
        if started.bucket == bucket && started.key == key && started.access_key == access_key {
            Ok(())
        } else {
            Err(s3_error!(NoSuchUpload))
        }
    }

    /// Stream one part to its file; its MD5 (the part's `ETag`).
    pub async fn put_part(
        &self,
        id: &str,
        number: i32,
        mut body: architect::blob::Reader,
    ) -> S3Result<String> {
        if !(1..=10_000).contains(&number) {
            return Err(s3_error!(InvalidArgument, "a part number is 1 to 10000"));
        }
        let dir = self.at(id)?;
        let part = dir.join(format!("{number}.part"));
        let io = |e: std::io::Error| s3_error!(InternalError, "{e}");
        let mut file = tokio::fs::File::create(&part).await.map_err(io)?;
        let mut md5 = md5::Md5::new();
        let mut buf = vec![0u8; 256 << 10];
        loop {
            let n = body.read(&mut buf).await.map_err(io)?;
            let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
                break;
            };
            file.write_all(chunk).await.map_err(io)?;
            md5.update(chunk);
        }
        file.flush().await.map_err(io)?;
        drop(file);
        tokio::fs::rename(&part, dir.join(number.to_string()))
            .await
            .map_err(io)?;
        Ok(hex(&md5.finalize()))
    }

    /// The parts `numbers`, in that order, as one stream.
    pub async fn joined(&self, id: &str, numbers: &[i32]) -> S3Result<architect::blob::Reader> {
        let dir = self.at(id)?;
        if numbers.is_empty() || numbers.windows(2).any(|w| w.first() >= w.get(1)) {
            return Err(s3_error!(InvalidPartOrder));
        }
        let mut files = std::collections::VecDeque::new();
        for n in numbers {
            let file = tokio::fs::File::open(dir.join(n.to_string()))
                .await
                .map_err(|_| s3_error!(InvalidPart))?;
            files.push_back(file);
        }
        Ok(Box::pin(Joined(files)))
    }

    pub async fn remove(&self, id: &str) {
        if let Ok(dir) = self.at(id) {
            let _ = tokio::fs::remove_dir_all(dir).await;
        }
    }
}

/// Files read one after another (flat, however many parts).
struct Joined(std::collections::VecDeque<tokio::fs::File>);

impl tokio::io::AsyncRead for Joined {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        loop {
            let Some(file) = self.0.front_mut() else {
                return std::task::Poll::Ready(Ok(()));
            };
            let before = buf.filled().len();
            match std::pin::Pin::new(file).poll_read(cx, buf) {
                std::task::Poll::Ready(Ok(())) if buf.filled().len() == before => {
                    self.0.pop_front();
                }
                other => return other,
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
