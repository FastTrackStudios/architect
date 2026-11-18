//! Shared memory: the frames' bytes where the GPU can't share them
//! ([`crate::wire::Transport::Shm`]).
//!
//! The program makes one file holding every slot ([`Shared::create`]),
//! maps it and sends its descriptor beside the buffers' message; the app
//! maps what it was sent ([`Shared::map`]) and uploads each frame from
//! it. On Linux the file is a `memfd`; on macOS a POSIX shared-memory
//! object, unlinked as soon as it is made, so nothing is left behind and
//! no other program can open it by name. Either way, only the descriptor
//! reaches it.
//!
//! The same on both platforms, so a program that shares its frames this
//! way (one on another wgpu than the app's, as Ignition's renderer is)
//! does not care where it runs.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

/// A shared mapping and the file it maps; unmapped when dropped.
#[derive(Debug)]
pub struct Shared {
    file: OwnedFd,
    ptr: std::ptr::NonNull<u8>,
    len: usize,
}

// SAFETY: the mapping is plain memory, valid while `self` lives; what
// writes into it across processes is ordered by the wire (a slot is
// written only while the other side doesn't hold it).
unsafe impl Send for Shared {}
// SAFETY: as above; `&Shared` hands out no `&mut` to the bytes.
unsafe impl Sync for Shared {}

impl Shared {
    /// A new file of `len` bytes, mapped for reading and writing.
    pub fn create(len: usize) -> io::Result<Self> {
        let file = anonymous()?;
        rustix::fs::ftruncate(
            &file,
            u64::try_from(len).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?,
        )?;
        Self::map_file(file, len, true)
    }

    /// The file `file` (sent by the other side), `len` bytes of it,
    /// mapped read-only (`writable: false`) or for writing too. Fails if
    /// the file is shorter than `len`.
    pub fn map(file: OwnedFd, len: usize, writable: bool) -> io::Result<Self> {
        let size = rustix::fs::fstat(&file)?.st_size;
        if u64::try_from(size).unwrap_or(0) < u64::try_from(len).unwrap_or(u64::MAX) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("the shared memory is {size} bytes, not {len}"),
            ));
        }
        Self::map_file(file, len, writable)
    }

    fn map_file(file: OwnedFd, len: usize, writable: bool) -> io::Result<Self> {
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an empty mapping",
            ));
        }
        let prot = if writable {
            rustix::mm::ProtFlags::READ | rustix::mm::ProtFlags::WRITE
        } else {
            rustix::mm::ProtFlags::READ
        };
        // SAFETY: a fresh shared mapping of a file we hold, `len` long and
        // no longer than the file (checked or just sized); it is unmapped
        // only in `drop`.
        let ptr = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                len,
                prot,
                rustix::mm::MapFlags::SHARED,
                &file,
                0,
            )
        }?;
        let ptr = std::ptr::NonNull::new(ptr.cast::<u8>())
            .ok_or_else(|| io::Error::other("mmap gave a null mapping"))?;
        Ok(Self { file, ptr, len })
    }

    /// Its length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Never: a mapping is at least one byte.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes `start..start + len`, if they are inside the mapping.
    ///
    /// # Safety
    ///
    /// The other process must not write these bytes while the slice
    /// lives (the wire's slot discipline: a slot is read only while the
    /// reader holds it).
    #[must_use]
    pub unsafe fn bytes(&self, start: usize, len: usize) -> Option<&[u8]> {
        let end = start.checked_add(len)?;
        if end > self.len {
            return None;
        }
        // SAFETY: inside the mapping (checked); the caller holds the slot.
        Some(unsafe { std::slice::from_raw_parts(self.ptr.as_ptr().add(start), len) })
    }

    /// The bytes `start..start + len` to write, if they are inside the
    /// mapping (and it was mapped writable).
    ///
    /// # Safety
    ///
    /// As [`Self::bytes`], and nothing else in this process reads or
    /// writes those bytes while the slice lives.
    #[must_use]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn bytes_mut(&self, start: usize, len: usize) -> Option<&mut [u8]> {
        let end = start.checked_add(len)?;
        if end > self.len {
            return None;
        }
        // SAFETY: inside the mapping (checked); exclusive by the caller's
        // promise.
        Some(unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr().add(start), len) })
    }
}

impl AsFd for Shared {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: mapped by `map_file` with this length; no slice of it
        // outlives `self`.
        let _ = unsafe { rustix::mm::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}

/// An anonymous file, close-on-exec.
#[cfg(target_os = "linux")]
fn anonymous() -> io::Result<OwnedFd> {
    Ok(rustix::fs::memfd_create(
        "architect-process-view",
        rustix::fs::MemfdFlags::CLOEXEC,
    )?)
}

/// An anonymous file, close-on-exec: a POSIX shared-memory object made
/// under a name no one else uses and unlinked at once.
#[cfg(target_os = "macos")]
fn anonymous() -> io::Result<OwnedFd> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // Shared-memory names are at most 31 bytes on macOS (PSHMNAMLEN).
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let name = format!("/apv.{}.{n}.{nanos:x}", std::process::id());
        match rustix::shm::open(
            name.as_str(),
            rustix::shm::OFlags::CREATE | rustix::shm::OFlags::EXCL | rustix::shm::OFlags::RDWR,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        ) {
            Ok(fd) => {
                let _ = rustix::shm::unlink(name.as_str());
                rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC)?;
                return Ok(fd);
            }
            Err(rustix::io::Errno::EXIST) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err(io::Error::other("no free shared-memory name"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    /// What one side writes, the other side's mapping of the same
    /// descriptor reads; and a file shorter than asked is refused.
    #[test]
    fn both_mappings_see_the_same_bytes() {
        let len = 3 * 64 * 48 * 4 + 5;
        let made = Shared::create(len).unwrap();
        // SAFETY: nothing else reads these bytes now.
        unsafe { made.bytes_mut(len - 5, 5) }
            .unwrap()
            .copy_from_slice(b"frame");
        let other = Shared::map(made.as_fd().try_clone_to_owned().unwrap(), len, false).unwrap();
        // SAFETY: nothing writes them now.
        assert_eq!(unsafe { other.bytes(len - 5, 5) }.unwrap(), b"frame");
        // SAFETY: as above.
        assert!(unsafe { other.bytes(len - 4, 5) }.is_none(), "past the end");
        let too_long = len.next_multiple_of(1 << 16) + (1 << 16);
        assert!(Shared::map(made.as_fd().try_clone_to_owned().unwrap(), too_long, false).is_err());
    }
}
