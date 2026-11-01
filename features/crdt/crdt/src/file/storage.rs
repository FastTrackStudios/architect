//! Where a document file's bytes live: a file on disk, or bytes in
//! memory (a browser handed a file, a test).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::FileError;
use crate::persistence::MaybeSendSync;

/// The bytes of one document file. A file only ever grows by whole
/// records at its end, is cut back to its last whole record after a
/// crash, or is replaced whole when it is folded.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Storage: MaybeSendSync + 'static {
    /// Every byte of the file (empty if there is none yet).
    async fn read(&self) -> Result<Vec<u8>, FileError>;

    /// Adds `bytes` at the end. On `Ok` they are durable: a crash after
    /// it returns does not lose them.
    async fn append(&self, bytes: &[u8]) -> Result<(), FileError>;

    /// Replaces the whole file with `bytes`, atomically: a crash leaves
    /// the old file or the new one, never a mix.
    async fn replace(&self, bytes: &[u8]) -> Result<(), FileError>;

    /// Cuts the file back to its first `len` bytes (a torn tail).
    async fn truncate(&self, len: u64) -> Result<(), FileError>;
}

/// A file held in memory: what a browser reads a file it was handed
/// into, and what tests crash on purpose. Clones share the bytes.
#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl MemoryStorage {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The file `bytes` are (`fetch`ed, picked from disk, sent by a peer).
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Arc::new(Mutex::new(bytes)),
        }
    }

    /// The file's bytes as they are now.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        architect::lock(&self.bytes).clone()
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Storage for MemoryStorage {
    async fn read(&self) -> Result<Vec<u8>, FileError> {
        Ok(self.bytes())
    }

    async fn append(&self, bytes: &[u8]) -> Result<(), FileError> {
        architect::lock(&self.bytes).extend_from_slice(bytes);
        Ok(())
    }

    async fn replace(&self, bytes: &[u8]) -> Result<(), FileError> {
        bytes.clone_into(&mut architect::lock(&self.bytes));
        Ok(())
    }

    async fn truncate(&self, len: u64) -> Result<(), FileError> {
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        architect::lock(&self.bytes).truncate(len);
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use disk::DiskStorage;

#[cfg(not(target_arch = "wasm32"))]
mod disk {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::{FileError, Storage};

    /// A document file on disk. Every append is synced before it returns,
    /// so a finished step survives the process being killed; a fold
    /// writes the new file beside the old and renames it over.
    ///
    /// One writer at a time: two processes appending to one file would
    /// interleave their records.
    ///
    /// Plain `std::fs` behind the async trait, like
    /// [`FilePersistence`](crate::FilePersistence): a step is small, and
    /// a blocking write of it is cheaper than an async file stack.
    #[derive(Debug)]
    pub struct DiskStorage {
        path: PathBuf,
        file: Mutex<File>,
    }

    // By value: an error-mapping adapter for `map_err`.
    #[allow(clippy::needless_pass_by_value)]
    fn io(e: std::io::Error) -> FileError {
        FileError::Storage(format!("fs: {e}"))
    }

    fn open(path: &Path) -> Result<File, FileError> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(io)
    }

    impl DiskStorage {
        /// The file at `path`, made (empty) if it is not there.
        pub fn open(path: impl Into<PathBuf>) -> Result<Self, FileError> {
            let path = path.into();
            let file = open(&path)?;
            Ok(Self {
                path,
                file: Mutex::new(file),
            })
        }

        #[must_use]
        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    #[async_trait]
    impl Storage for DiskStorage {
        async fn read(&self) -> Result<Vec<u8>, FileError> {
            let mut file = architect::lock(&self.file);
            let mut out = Vec::new();
            file.seek(SeekFrom::Start(0)).map_err(io)?;
            file.read_to_end(&mut out).map_err(io)?;
            drop(file);
            Ok(out)
        }

        async fn append(&self, bytes: &[u8]) -> Result<(), FileError> {
            let mut file = architect::lock(&self.file);
            file.seek(SeekFrom::End(0)).map_err(io)?;
            file.write_all(bytes).map_err(io)?;
            let synced = file.sync_data().map_err(io);
            drop(file);
            synced
        }

        async fn replace(&self, bytes: &[u8]) -> Result<(), FileError> {
            let mut file = architect::lock(&self.file);
            let mut tmp_name = self.path.clone().into_os_string();
            tmp_name.push(".tmp");
            let tmp = PathBuf::from(tmp_name);
            {
                let mut out = File::create(&tmp).map_err(io)?;
                out.write_all(bytes).map_err(io)?;
                out.sync_all().map_err(io)?;
            }
            std::fs::rename(&tmp, &self.path).map_err(io)?;
            // The rename itself is durable once the directory is synced.
            #[cfg(unix)]
            if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
                File::open(dir).and_then(|d| d.sync_all()).map_err(io)?;
            }
            *file = open(&self.path)?;
            drop(file);
            Ok(())
        }

        async fn truncate(&self, len: u64) -> Result<(), FileError> {
            let file = architect::lock(&self.file);
            file.set_len(len).map_err(io)?;
            let synced = file.sync_all().map_err(io);
            drop(file);
            synced
        }
    }
}
