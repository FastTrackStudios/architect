//! The socket a view's messages travel on: one end of a Unix socket
//! pair carrying one message at a time, in order, never merged, with file
//! descriptors passed beside a message (`SCM_RIGHTS`).
//!
//! On Linux the pair is `SOCK_SEQPACKET`: one message per packet. macOS
//! has no `SOCK_SEQPACKET` for Unix sockets, and its `SOCK_DGRAM` pair
//! never says that the peer is gone (a reader blocked in `recv` stays
//! blocked, `poll` stays quiet, after the program is killed), so there it
//! is `SOCK_STREAM`, each message framed by its length (a little-endian
//! `u32`) and its descriptors sent with the frame's first bytes. A frame
//! is read whole, never past its end, so the descriptors stay with their
//! message; sends are serialised, so two threads' frames never interleave.
//! Either way [`Socket::recv`] says `None` once the peer closed its end
//! and everything it sent is read, and a send blocks only while the
//! peer's queue is full.
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
pub struct Socket {
    fd: OwnedFd,
    /// macOS: one frame written at a time.
    #[cfg(target_os = "macos")]
    sending: std::sync::Mutex<()>,
    /// macOS: one frame read at a time.
    #[cfg(target_os = "macos")]
    reading: std::sync::Mutex<()>,
}

impl Socket {
    const fn wrap(fd: OwnedFd) -> Self {
        Self {
            fd,
            #[cfg(target_os = "macos")]
            sending: std::sync::Mutex::new(()),
            #[cfg(target_os = "macos")]
            reading: std::sync::Mutex::new(()),
        }
    }

    /// A connected pair: the host's end and the guest's, both closed in
    /// any program this process starts. [`hand_to`] gives the guest's to
    /// one program alone.
    pub fn pair() -> io::Result<(Self, OwnedFd)> {
        #[cfg(target_os = "linux")]
        let (host, guest) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )?;
        #[cfg(target_os = "macos")]
        let (host, guest) = {
            let (host, guest) = socketpair(
                AddressFamily::UNIX,
                SocketType::STREAM,
                SocketFlags::empty(),
                None,
            )?;
            for end in [&host, &guest] {
                rustix::io::fcntl_setfd(end, rustix::io::FdFlags::CLOEXEC)?;
                stream_options(end)?;
            }
            (host, guest)
        };
        Ok((Self::wrap(host), guest))
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
        Ok(Self::wrap(fd))
    }

    /// One message with `fds` beside it. Blocks only if the peer's
    /// queue is full.
    pub fn send(&self, bytes: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<()> {
        if bytes.len() > MAX_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "message too large",
            ));
        }
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        if !fds.is_empty() && !control.push(SendAncillaryMessage::ScmRights(fds)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "too many descriptors",
            ));
        }
        #[cfg(target_os = "linux")]
        {
            let iov = [io::IoSlice::new(bytes)];
            sendmsg(&self.fd, &iov, &mut control, SendFlags::NOSIGNAL)?;
            Ok(())
        }
        #[cfg(target_os = "macos")]
        {
            let len = u32::try_from(bytes.len())
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            let mut frame = Vec::with_capacity(bytes.len().saturating_add(4));
            frame.extend_from_slice(&len.to_le_bytes());
            frame.extend_from_slice(bytes);
            let _one = self
                .sending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // The descriptors with the first bytes; the rest, if the
            // socket took only part, after them. (`SO_NOSIGPIPE` is set.)
            let mut sent = loop {
                match sendmsg(
                    &self.fd,
                    &[io::IoSlice::new(&frame)],
                    &mut control,
                    SendFlags::empty(),
                ) {
                    Ok(n) => break n,
                    Err(rustix::io::Errno::INTR) => {}
                    Err(e) => return Err(e.into()),
                }
            };
            while let Some(rest) = frame.get(sent..).filter(|r| !r.is_empty()) {
                match rustix::net::send(&self.fd, rest, SendFlags::empty()) {
                    Ok(n) => sent = sent.saturating_add(n),
                    Err(rustix::io::Errno::INTR) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(())
        }
    }

    /// The next message and the descriptors beside it; `None` when the
    /// peer has closed its end. Blocks until one arrives.
    #[cfg(target_os = "linux")]
    pub fn recv(&self) -> io::Result<Option<(Vec<u8>, Vec<OwnedFd>)>> {
        let mut buf = vec![0u8; MAX_MESSAGE];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let mut iov = [io::IoSliceMut::new(&mut buf)];
        let got = recvmsg(&self.fd, &mut iov, &mut control, RecvFlags::CMSG_CLOEXEC)?;
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

    /// The next message and the descriptors beside it; `None` when the
    /// peer has closed its end. Blocks until one arrives.
    #[cfg(target_os = "macos")]
    pub fn recv(&self) -> io::Result<Option<(Vec<u8>, Vec<OwnedFd>)>> {
        let _one = self
            .reading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut fds = Vec::new();
        let mut header = [0u8; 4];
        if !self.read_exact(&mut header, &mut fds)? {
            return Ok(None);
        }
        let len = usize::try_from(u32::from_le_bytes(header))
            .ok()
            .filter(|l| *l <= MAX_MESSAGE)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a frame too large"))?;
        let mut buf = vec![0u8; len];
        if !self.read_exact(&mut buf, &mut fds)? && len > 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(Some((buf, fds)))
    }

    /// Fills `buf` from the stream, gathering any descriptors; false if
    /// the peer had closed before the first byte (an error if it closes
    /// part way).
    #[cfg(target_os = "macos")]
    fn read_exact(&self, buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> io::Result<bool> {
        let mut filled = 0usize;
        while let Some(rest) = buf.get_mut(filled..).filter(|r| !r.is_empty()) {
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
            let mut control = RecvAncillaryBuffer::new(&mut space);
            let got = match recvmsg(
                &self.fd,
                &mut [io::IoSliceMut::new(rest)],
                &mut control,
                RecvFlags::empty(),
            ) {
                Ok(got) => got.bytes,
                Err(rustix::io::Errno::INTR) => continue,
                Err(rustix::io::Errno::CONNRESET) => 0,
                Err(e) => return Err(e.into()),
            };
            for message in control.drain() {
                if let RecvAncillaryMessage::ScmRights(rights) = message {
                    for fd in rights {
                        // No `MSG_CMSG_CLOEXEC` here: set it before any
                        // program this process starts can inherit it.
                        rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC)?;
                        fds.push(fd);
                    }
                }
            }
            if got == 0 {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            filled = filled.saturating_add(got);
        }
        Ok(true)
    }

    /// Whether a message is waiting, waiting up to `timeout` (`None`:
    /// for ever) for one. A closed peer counts as waiting: the next
    /// [`Self::recv`] says so.
    pub fn wait(&self, timeout: Option<Duration>) -> io::Result<bool> {
        let mut fds = [rustix::event::PollFd::new(
            &self.fd,
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

/// A stream socket's options on macOS: room for a queue of
/// [`MAX_MESSAGE`] frames, and no SIGPIPE when the peer is gone.
#[cfg(target_os = "macos")]
fn stream_options(fd: &OwnedFd) -> io::Result<()> {
    const ROOM: usize = MAX_MESSAGE * 16;
    rustix::net::sockopt::set_socket_send_buffer_size(fd, ROOM)?;
    rustix::net::sockopt::set_socket_recv_buffer_size(fd, ROOM)?;
    rustix::net::sockopt::set_socket_nosigpipe(fd, true)?;
    Ok(())
}

/// Gives `guest` (a [`Socket::pair`]'s guest end) to the program
/// `command` starts, in [`FD_ENV`].
///
/// On macOS it also names the host's `IOSurface` rendezvous
/// (`crate::surface::ENV`), when this process has one, so the program
/// can share its frames on the GPU.
///
/// The end stays close-on-exec in this process, so no other program it
/// starts meanwhile (another view's, a process extension's, on another
/// thread) inherits it and holds the socket open after this program dies;
/// only the child, between fork and exec, clears the flag on its copy.
pub fn hand_to(command: &mut std::process::Command, guest: &OwnedFd) {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::process::CommandExt as _;
    let raw = guest.as_raw_fd();
    command.env(FD_ENV, raw.to_string());
    #[cfg(target_os = "macos")]
    if let Some(name) = crate::surface::Rendezvous::get().map(crate::surface::Rendezvous::name) {
        command.env(crate::surface::ENV, name);
    }
    // SAFETY: runs in the child after fork: one fcntl on a descriptor the
    // child has (inherited from this process), async-signal-safe.
    unsafe {
        command.pre_exec(move || {
            let fd = BorrowedFd::borrow_raw(raw);
            rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::empty()).map_err(io::Error::from)
        });
    }
}

impl AsFd for Socket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Seek as _, Write as _};

    #[test]
    fn a_message_and_its_descriptors_arrive_together_in_order() {
        let (host, guest) = Socket::pair().unwrap();
        let guest = Socket::wrap(guest);
        let mut file = tempfile::tempfile().unwrap();
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

    /// The guest's end reaches the program it is handed to and no other:
    /// a sibling started meanwhile must not hold it (it would keep the
    /// socket open after the program dies, hiding its death).
    /// Where a shell sees its own descriptors.
    #[cfg(target_os = "linux")]
    const FDS: &str = "/proc/self/fd";
    #[cfg(target_os = "macos")]
    const FDS: &str = "/dev/fd";

    /// A message as large as the wire allows arrives whole, and many
    /// queue without a reader (macOS's default send space is 2 KiB).
    #[test]
    fn the_largest_message_fits_and_many_queue() {
        let (host, guest) = Socket::pair().unwrap();
        let guest = Socket::wrap(guest);
        let big = vec![7u8; MAX_MESSAGE];
        guest.send(&big, &[]).unwrap();
        for _ in 0..64 {
            guest.send(b"tick", &[]).unwrap();
        }
        let (bytes, _) = host.recv().unwrap().unwrap();
        assert_eq!(bytes, big);
        for _ in 0..64 {
            assert_eq!(host.recv().unwrap().unwrap().0, b"tick");
        }
    }

    /// A reader already waiting when the peer goes is woken (a macOS
    /// datagram pair never wakes it: why macOS frames a stream).
    #[test]
    fn a_reader_waiting_when_the_peer_dies_is_woken() {
        let (host, guest) = Socket::pair().unwrap();
        let reader = std::thread::spawn(move || host.recv().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(100));
        drop(guest);
        let woke = std::time::Instant::now();
        while !reader.is_finished() {
            assert!(
                woke.elapsed() < Duration::from_secs(5),
                "the reader was never woken"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(reader.join().unwrap(), "the end, not a message");
    }

    /// A program that dies mid-conversation: what it sent is read, then
    /// the end, then sending to it fails without a signal.
    #[test]
    fn a_closed_peer_reads_as_the_end_after_its_messages() {
        let (host, guest) = Socket::pair().unwrap();
        let guest = Socket::wrap(guest);
        guest.send(b"last words", &[]).unwrap();
        drop(guest);
        assert_eq!(host.recv().unwrap().unwrap().0, b"last words");
        assert!(host.recv().unwrap().is_none());
        assert!(host.send(b"anyone?", &[]).is_err());
    }

    #[test]
    fn only_the_program_handed_the_end_inherits_it() {
        use std::os::fd::AsRawFd as _;
        let (_host, guest) = Socket::pair().unwrap();
        let raw = guest.as_raw_fd();
        let has = |cmd: &mut std::process::Command| {
            let out = cmd
                .arg("-c")
                .arg(format!("test -e {FDS}/{raw} && echo yes || echo no"))
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };
        assert_eq!(
            has(&mut std::process::Command::new("sh")),
            "no",
            "a sibling"
        );
        let mut handed = std::process::Command::new("sh");
        hand_to(&mut handed, &guest);
        assert_eq!(has(&mut handed), "yes", "the program handed it");
    }
}
