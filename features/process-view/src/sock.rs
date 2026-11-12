//! The socket a view's messages travel on: one end of a
//! `SOCK_SEQPACKET` pair (one message per datagram, in order, never
//! merged), with file descriptors passed beside a message
//! (`SCM_RIGHTS`).
//!
//! The host makes the pair when it starts the program and hands the
//! guest its end as an inherited descriptor named by [`FD_ENV`]; nothing
//! is on the filesystem, so no other program can connect to it.

use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd};
use std::time::Duration;

use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, SocketFlags, SocketType, recvmsg, sendmsg, socketpair,
};

use crate::wire::MAX_MESSAGE;

/// The environment variable naming the guest's end of the socket (a
/// descriptor number).
pub const FD_ENV: &str = "ARCHITECT_VIEW_FD";

/// At most this many descriptors beside one message.
pub const MAX_FDS: usize = 8;

/// One end of a view's socket.
#[derive(Debug)]
pub struct Socket(OwnedFd);

impl Socket {
    /// A connected pair: the host's end (closed in any program it
    /// starts) and the guest's (inherited by the program).
    pub fn pair() -> io::Result<(Self, OwnedFd)> {
        let (host, guest) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        // The guest's end is inherited across exec: clear its CLOEXEC.
        rustix::io::fcntl_setfd(&guest, rustix::io::FdFlags::empty())?;
        Ok((Self(host), guest))
    }

    /// The end a program was handed: [`FD_ENV`]'s descriptor.
    pub fn from_env() -> io::Result<Self> {
        let raw: RawFd = std::env::var(FD_ENV)
            .ok()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{FD_ENV} not set")))?;
        // SAFETY: the host passed this descriptor for us alone; nothing
        // else in this process owns it.
        let fd = unsafe { <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(raw) };
        rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC)?;
        Ok(Self(fd))
    }

    /// One message with `fds` beside it. Blocks only if the peer's
    /// queue is full.
    pub fn send(&self, bytes: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<()> {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        if !fds.is_empty() && !control.push(SendAncillaryMessage::ScmRights(fds)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "too many descriptors",
            ));
        }
        let iov = [io::IoSlice::new(bytes)];
        sendmsg(&self.0, &iov, &mut control, SendFlags::NOSIGNAL)?;
        Ok(())
    }

    /// The next message and the descriptors beside it; `None` when the
    /// peer has closed its end. Blocks until one arrives.
    pub fn recv(&self) -> io::Result<Option<(Vec<u8>, Vec<OwnedFd>)>> {
        let mut buf = vec![0u8; MAX_MESSAGE];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let mut iov = [io::IoSliceMut::new(&mut buf)];
        let got = recvmsg(&self.0, &mut iov, &mut control, RecvFlags::CMSG_CLOEXEC)?;
        let mut fds = Vec::new();
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(rights) = message {
                fds.extend(rights);
            }
        }
        if got.bytes == 0 && fds.is_empty() {
            return Ok(None);
        }
        buf.truncate(got.bytes);
        Ok(Some((buf, fds)))
    }

    /// Whether a message is waiting, waiting up to `timeout` (`None`:
    /// for ever) for one. A closed peer counts as waiting: the next
    /// [`Self::recv`] says so.
    pub fn wait(&self, timeout: Option<Duration>) -> io::Result<bool> {
        let mut fds = [rustix::event::PollFd::new(
            &self.0,
            rustix::event::PollFlags::IN,
        )];
        let timeout = timeout.map(|t| rustix::time::Timespec {
            tv_sec: i64::try_from(t.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: i64::from(t.subsec_nanos()),
        });
        match rustix::event::poll(&mut fds, timeout.as_ref()) {
            Ok(n) => Ok(n > 0),
            Err(rustix::io::Errno::INTR) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

impl AsFd for Socket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Seek as _, Write as _};

    #[test]
    fn a_message_and_its_descriptors_arrive_together_in_order() {
        let (host, guest) = Socket::pair().unwrap();
        let guest = Socket(guest);
        let mut file = std::fs::File::from(
            rustix::fs::memfd_create("view-test", rustix::fs::MemfdFlags::CLOEXEC).unwrap(),
        );
        file.write_all(b"pixels").unwrap();
        guest.send(b"one", &[file.as_fd()]).unwrap();
        guest.send(b"two", &[]).unwrap();
        let (bytes, fds) = host.recv().unwrap().unwrap();
        assert_eq!(bytes, b"one");
        assert_eq!(fds.len(), 1);
        let mut shared = std::fs::File::from(fds.into_iter().next().unwrap());
        shared.rewind().unwrap();
        let mut read = String::new();
        shared.read_to_string(&mut read).unwrap();
        assert_eq!(read, "pixels");
        let (bytes, fds) = host.recv().unwrap().unwrap();
        assert_eq!((bytes.as_slice(), fds.len()), (&b"two"[..], 0));
        drop(guest);
        assert!(host.wait(Some(Duration::from_millis(100))).unwrap());
        assert!(host.recv().unwrap().is_none());
    }
}
