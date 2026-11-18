//! macOS: frames the GPU shares between two processes, as `IOSurface`s
//! ([`crate::wire::Transport::IoSurface`]).
//!
//! An `IOSurface` is memory the window server and every GPU device on the
//! machine can use as a texture: the program renders into one (a Metal
//! texture made over it, [`IoSurface::metal_texture`]) and the app's
//! device makes its own texture over the same surface, so a frame is
//! never copied between them. It is the dma-buf of macOS.
//!
//! **How a surface travels.** A surface goes to another process as a Mach
//! port (`IOSurfaceCreateMachPort`), and a Mach port cannot travel on a
//! Unix socket. So the app has one Mach port of its own, the
//! [`Rendezvous`], registered with launchd under a name only it uses
//! (`dev.architect.process-view.<pid>`), and passes that name to each
//! program it starts ([`ENV`], beside the socket's [`crate::sock::FD_ENV`]).
//! The program looks it up ([`Sender::from_env`]) and, for each set of
//! buffers, sends one message holding every slot's surface and the set's
//! id ([`Sender::send`]), then the buffers' message on the socket; the
//! app's session takes the surfaces of that id ([`Rendezvous::take`]).
//!
//! **Who may send.** Anyone in the person's session can look the name up
//! and send to it, so the rendezvous keeps a surface only from a program
//! the app started and is still expecting ([`Rendezvous::expect`]): the
//! kernel stamps each message with its sender's pid (the audit trailer),
//! which a sender cannot forge. Anything else is dropped, its rights
//! released.
//!
//! **Where it can't.** A sandboxed app cannot register a name: then
//! [`Rendezvous::get`] is `None`, the app says hello without GPU-shared
//! frames, and the program sends shared memory, as on another GPU on
//! Linux.
//!
//! The format is `RGBA8` (`'RGBA'`), rows aligned as `IOSurface` wants,
//! one plane. Synchronisation is the wire's, as for dma-bufs: the program
//! waits for its GPU before it says a frame is ready, and writes only a
//! slot the app released.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::CString;
use std::io;
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use mach2::bootstrap::{bootstrap_check_in, bootstrap_look_up, bootstrap_port};
use mach2::kern_return::{KERN_SUCCESS, kern_return_t};
use mach2::mach_port::mach_port_deallocate;
use mach2::message::{
    MACH_MSG_PORT_DESCRIPTOR, MACH_MSG_SUCCESS, MACH_MSG_TIMEOUT_NONE, MACH_MSG_TYPE_COPY_SEND,
    MACH_MSG_TYPE_MOVE_SEND, MACH_MSGH_BITS, MACH_MSGH_BITS_COMPLEX, MACH_RCV_MSG,
    MACH_RCV_TRAILER_AUDIT, MACH_SEND_MSG, MACH_SEND_TIMEOUT, mach_msg, mach_msg_audit_trailer_t,
    mach_msg_body_t, mach_msg_destroy, mach_msg_header_t, mach_msg_port_descriptor_t,
    mach_msg_size_t,
};
use mach2::port::{MACH_PORT_NULL, mach_port_t};
use mach2::traps::mach_task_self;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

use crate::wire::MAX_SLOTS;

/// The environment variable naming the app's rendezvous (a launchd
/// service name) to a program it starts.
pub const ENV: &str = "ARCHITECT_VIEW_SURFACES";

/// `'RGBA'`: four bytes a pixel, red first.
const PIXEL_FORMAT: i32 = i32::from_be_bytes(*b"RGBA");

/// One port per slot in a message.
#[expect(
    clippy::as_conversions,
    reason = "u8 to usize widens; From is not const"
)]
const PORTS: usize = MAX_SLOTS as usize;

/// The message's id: `APV1`.
const MESSAGE_ID: i32 = i32::from_be_bytes(*b"APV1");

/// How long a program waits for the app to take its surfaces before the
/// send fails (the app's port queue is full: it stopped reading).
const SEND_WITHIN: Duration = Duration::from_secs(2);

/// Sets of buffers kept for one program, oldest dropped first: a program
/// that announces without the app taking them can't grow the app.
const KEPT: usize = 4;

/// An `IOSurface`, one frame's memory.
#[derive(Debug, Clone)]
pub struct IoSurface(CFRetained<IOSurfaceRef>);

// SAFETY: IOSurface is a thread-safe CoreFoundation type: retaining,
// releasing and reading its properties are safe from any thread, and the
// pixels are ordered by the wire, not by this handle.
#[expect(
    clippy::non_send_fields_in_send_ty,
    reason = "CFRetained<IOSurfaceRef> is not Send in objc2's types, but IOSurface is thread-safe"
)]
unsafe impl Send for IoSurface {}
// SAFETY: as above; `&IoSurface` only reads properties.
unsafe impl Sync for IoSurface {}

impl IoSurface {
    /// A new `width` × `height` RGBA8 surface.
    pub fn new(width: u32, height: u32) -> io::Result<Self> {
        if width == 0 || height == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an empty surface",
            ));
        }
        let row = usize::try_from(width)
            .ok()
            .and_then(|w| w.checked_mul(4))
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: the property names are the framework's constants.
        let (k_width, k_height, k_element, k_row, k_format) = unsafe {
            (
                objc2_io_surface::kIOSurfaceWidth,
                objc2_io_surface::kIOSurfaceHeight,
                objc2_io_surface::kIOSurfaceBytesPerElement,
                objc2_io_surface::kIOSurfaceBytesPerRow,
                objc2_io_surface::kIOSurfacePixelFormat,
            )
        };
        let stride = IOSurfaceRef::align_property(k_row, row);
        let number = |v: i64| CFNumber::new_i64(v);
        let (w, h, bpe, bpr, format) = (
            number(i64::from(width)),
            number(i64::from(height)),
            number(4),
            number(i64::try_from(stride).unwrap_or(i64::MAX)),
            number(i64::from(PIXEL_FORMAT)),
        );
        let keys: [&CFString; 5] = [k_width, k_height, k_element, k_row, k_format];
        let values: [&CFType; 5] = [&w, &h, &bpe, &bpr, &format];
        let properties = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
        // SAFETY: a dictionary of IOSurface's own property keys, each with
        // a CFNumber value of the type it documents.
        let surface = unsafe { IOSurfaceRef::new(properties.as_opaque()) }
            .ok_or_else(|| io::Error::other("IOSurfaceCreate failed"))?;
        Ok(Self(surface))
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        u32::try_from(self.0.width()).unwrap_or(0)
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        u32::try_from(self.0.height()).unwrap_or(0)
    }

    /// Bytes from one row to the next.
    #[must_use]
    pub fn stride(&self) -> u32 {
        u32::try_from(self.0.bytes_per_row()).unwrap_or(0)
    }

    /// The surface's system-wide id (for logs; it is not how a surface
    /// travels).
    #[must_use]
    pub fn id(&self) -> u32 {
        self.0.id()
    }

    /// The surface itself.
    #[must_use]
    pub fn surface(&self) -> &IOSurfaceRef {
        &self.0
    }

    /// Its pixels read on the CPU, rows packed (`width * 4` bytes each),
    /// as the GPU last left them: for a test, or a capture of a frame.
    /// `None` if the surface can't be locked.
    #[must_use]
    pub fn pixels(&self) -> Option<Vec<u8>> {
        let row = usize::try_from(self.width()).ok()?.checked_mul(4)?;
        let height = usize::try_from(self.height()).ok()?;
        let stride = self.0.bytes_per_row();
        // Checked before the lock, so nothing below returns holding it.
        let total = row.checked_mul(height)?;
        stride.checked_mul(height)?;
        if stride < row {
            return None;
        }
        // SAFETY: a read-only lock, released below; no seed asked for.
        let locked = unsafe {
            self.0.lock(
                objc2_io_surface::IOSurfaceLockOptions::ReadOnly,
                std::ptr::null_mut(),
            )
        };
        if locked != KERN_SUCCESS {
            return None;
        }
        let base = self.0.base_address().cast::<u8>();
        let mut out = Vec::with_capacity(total);
        for y in 0..height {
            // SAFETY: row `y` of a locked surface `height` rows of
            // `stride` bytes (at least `row` of them used); `y * stride`
            // was checked above.
            let line = unsafe {
                std::slice::from_raw_parts(base.as_ptr().add(y.wrapping_mul(stride)), row)
            };
            out.extend_from_slice(line);
        }
        // SAFETY: the lock taken above.
        unsafe {
            self.0.unlock(
                objc2_io_surface::IOSurfaceLockOptions::ReadOnly,
                std::ptr::null_mut(),
            )
        };
        Some(out)
    }

    /// A Metal texture of `device` over this surface (`RGBA8Unorm`, one
    /// level), which the device can render into, copy into and out of,
    /// and sample. `None` if Metal refuses.
    ///
    /// Plain objc2 types, so a program on another wgpu than this crate's
    /// (whose Metal backend is the same objc2) wraps the texture in its
    /// own (`wgpu_hal::metal::Device::texture_from_raw`).
    #[must_use]
    pub fn metal_texture(
        &self,
        device: &ProtocolObject<dyn MTLDevice>,
    ) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
        // SAFETY: a plain 2D descriptor; its size is the surface's.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::RGBA8Unorm,
                self.0.width(),
                self.0.height(),
                false,
            )
        };
        descriptor.setUsage(
            MTLTextureUsage::ShaderRead
                | MTLTextureUsage::ShaderWrite
                | MTLTextureUsage::RenderTarget,
        );
        // A surface's texture is shared memory on unified memory, managed
        // (a copy each side, kept in step) on a discrete GPU.
        descriptor.setStorageMode(if device.hasUnifiedMemory() {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Managed
        });
        device.newTextureWithDescriptor_iosurface_plane(&descriptor, &self.0, 0)
    }
}

// ── The message ──────────────────────────────────────────────────────

/// One set of buffers' surfaces: a Mach message with one port per slot
/// (unused ones null) and the set's id.
#[repr(C)]
#[derive(Clone, Copy)]
struct Message {
    header: mach_msg_header_t,
    body: mach_msg_body_t,
    ports: [mach_msg_port_descriptor_t; PORTS],
    /// The buffers' id, low and high halves (a `u64` would be padded).
    buffers: [u32; 2],
    count: u32,
}

/// What the rendezvous receives: the message and the kernel's trailer.
#[repr(C)]
#[derive(Clone, Copy)]
struct Received {
    message: Message,
    trailer: mach_msg_audit_trailer_t,
}

/// `MACH_RCV_TRAILER_TYPE(MACH_MSG_TRAILER_FORMAT_0)`.
const TRAILER_FORMAT_0: i32 = 0;

/// `MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT)`.
const fn trailer_elements(elements: u32) -> i32 {
    #[allow(clippy::cast_possible_wrap, clippy::as_conversions)]
    let shifted = ((elements & 0xf) << 24) as i32;
    shifted
}

fn size_of<T>() -> mach_msg_size_t {
    mach_msg_size_t::try_from(std::mem::size_of::<T>()).unwrap_or(mach_msg_size_t::MAX)
}

fn mach_error(what: &str, kr: kern_return_t) -> io::Error {
    io::Error::other(format!("{what}: Mach error {kr:#x}"))
}

/// `mach_port_set_attributes`, which `mach2` doesn't declare.
mod ffi {
    use mach2::kern_return::kern_return_t;
    use mach2::port::mach_port_t;
    unsafe extern "C" {
        pub fn mach_port_set_attributes(
            task: mach_port_t,
            name: mach_port_t,
            flavor: i32,
            info: *const i32,
            count: u32,
        ) -> kern_return_t;
    }
    /// `MACH_PORT_LIMITS_INFO`.
    pub const LIMITS_INFO: i32 = 1;
}

// ── The program's side ───────────────────────────────────────────────

/// A program's way to send its surfaces to the app that started it.
#[derive(Debug)]
pub struct Sender {
    port: mach_port_t,
}

impl Sender {
    /// The app's rendezvous, named in [`ENV`]: `None` if the app named
    /// none (it can't take surfaces: send shared memory).
    pub fn from_env() -> io::Result<Option<Self>> {
        let Ok(name) = std::env::var(ENV) else {
            return Ok(None);
        };
        let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let mut port: mach_port_t = MACH_PORT_NULL;
        // SAFETY: a lookup in this task's own bootstrap namespace, into a
        // port name we own from then on.
        let kr = unsafe { bootstrap_look_up(bootstrap_port, name.as_ptr(), &raw mut port) };
        if kr != KERN_SUCCESS {
            return Err(mach_error("looking up the app's rendezvous", kr));
        }
        Ok(Some(Self { port }))
    }

    /// Sends the surfaces of buffers `buffers` (one per slot, at most
    /// [`MAX_SLOTS`]); send the buffers' message on the socket after it.
    pub fn send(&self, buffers: u64, surfaces: &[IoSurface]) -> io::Result<()> {
        if surfaces.len() > usize::from(MAX_SLOTS) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "too many surfaces",
            ));
        }
        let mut ports =
            [mach_msg_port_descriptor_t::new(MACH_PORT_NULL, MACH_MSG_TYPE_COPY_SEND); PORTS];
        for (slot, surface) in ports.iter_mut().zip(surfaces) {
            // A send right of our own, moved into the message.
            *slot = mach_msg_port_descriptor_t::new(
                surface.0.create_mach_port(),
                MACH_MSG_TYPE_MOVE_SEND,
            );
        }
        #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
        let mut message = Message {
            header: mach_msg_header_t {
                msgh_bits: MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX,
                msgh_size: size_of::<Message>(),
                msgh_remote_port: self.port,
                msgh_local_port: MACH_PORT_NULL,
                msgh_voucher_port: MACH_PORT_NULL,
                msgh_id: MESSAGE_ID,
            },
            body: mach_msg_body_t {
                msgh_descriptor_count: u32::from(MAX_SLOTS),
            },
            ports,
            buffers: [buffers as u32, (buffers >> 32) as u32],
            count: u32::try_from(surfaces.len()).unwrap_or(0),
        };
        let within = u32::try_from(SEND_WITHIN.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: a well-formed complex message of `msgh_size` bytes,
        // sent to a send right we hold.
        let kr = unsafe {
            mach_msg(
                &raw mut message.header,
                MACH_SEND_MSG | MACH_SEND_TIMEOUT,
                size_of::<Message>(),
                0,
                MACH_PORT_NULL,
                within,
                MACH_PORT_NULL,
            )
        };
        if kr != MACH_MSG_SUCCESS {
            // Not sent: the rights are still ours.
            for p in &message.ports {
                if p.name != MACH_PORT_NULL {
                    // SAFETY: a send right this task holds.
                    unsafe { mach_port_deallocate(mach_task_self(), p.name) };
                }
            }
            return Err(mach_error("sending the surfaces", kr));
        }
        Ok(())
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // SAFETY: the send right `from_env` looked up, ours alone.
        unsafe { mach_port_deallocate(mach_task_self(), self.port) };
    }
}

// ── The app's side ───────────────────────────────────────────────────

#[derive(Default)]
struct Arrivals {
    /// The programs that may send (their pids).
    expected: HashSet<u32>,
    /// What each sent, by buffers id, oldest first.
    sets: HashMap<u32, VecDeque<(u64, Vec<IoSurface>)>>,
}

/// The app's port programs send their surfaces to: one per app process,
/// read by a thread of its own.
pub struct Rendezvous {
    name: String,
    arrivals: Mutex<Arrivals>,
    arrived: Condvar,
}

impl std::fmt::Debug for Rendezvous {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rendezvous")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Rendezvous {
    /// This process's rendezvous, registered the first time it is asked
    /// for; `None` where it can't be (a sandboxed app), and then frames go
    /// by shared memory.
    pub fn get() -> Option<&'static Self> {
        static ONE: OnceLock<Option<&'static Rendezvous>> = OnceLock::new();
        *ONE.get_or_init(|| match Self::register() {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("process views: no IOSurface rendezvous ({e}); frames by shared memory");
                None
            }
        })
    }

    fn register() -> io::Result<&'static Self> {
        let name = format!("dev.architect.process-view.{}", std::process::id());
        let c_name =
            CString::new(name.clone()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let mut port: mach_port_t = MACH_PORT_NULL;
        // SAFETY: registers a name in this task's bootstrap namespace and
        // gives us its receive right.
        let kr = unsafe { bootstrap_check_in(bootstrap_port, c_name.as_ptr(), &raw mut port) };
        if kr != KERN_SUCCESS {
            return Err(mach_error("registering the rendezvous", kr));
        }
        // Room for a burst (each resize announces a set): the default
        // queue is five messages.
        let qlimit: i32 = 64;
        // SAFETY: the limits flavour of the receive right we were given.
        unsafe {
            ffi::mach_port_set_attributes(
                mach_task_self(),
                port,
                ffi::LIMITS_INFO,
                &raw const qlimit,
                1,
            )
        };
        let rendezvous: &'static Self = Box::leak(Box::new(Self {
            name,
            arrivals: Mutex::new(Arrivals::default()),
            arrived: Condvar::new(),
        }));
        std::thread::Builder::new()
            .name("process-view surfaces".into())
            .spawn(move || rendezvous.serve(port))?;
        Ok(rendezvous)
    }

    /// The launchd name programs look up ([`ENV`]).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Takes surfaces from the program `pid` (one the app just started).
    pub fn expect(&self, pid: u32) {
        lock(&self.arrivals).expected.insert(pid);
    }

    /// No more from `pid` (its session ended); what it sent is released.
    pub fn forget(&self, pid: u32) {
        let mut arrivals = lock(&self.arrivals);
        arrivals.expected.remove(&pid);
        arrivals.sets.remove(&pid);
    }

    /// The surfaces program `pid` sent for buffers `buffers`, waiting up
    /// to `within` for them (the program sends them just before the
    /// buffers' message, but on another channel). Older sets of that
    /// program are dropped: it moved on from them.
    #[must_use]
    pub fn take(&self, pid: u32, buffers: u64, within: Duration) -> Option<Vec<IoSurface>> {
        let until = Instant::now().checked_add(within)?;
        let mut arrivals = lock(&self.arrivals);
        loop {
            if let Some(sets) = arrivals.sets.get_mut(&pid)
                && let Some(at) = sets.iter().position(|(id, _)| *id == buffers)
            {
                let taken = sets.remove(at).map(|(_, s)| s);
                sets.drain(..at);
                return taken;
            }
            let left = until.checked_duration_since(Instant::now())?;
            if left.is_zero() {
                return None;
            }
            arrivals = self
                .arrived
                .wait_timeout(arrivals, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The thread: every message, for ever.
    fn serve(&self, port: mach_port_t) {
        loop {
            // SAFETY: a zeroed buffer of plain integers is a valid value.
            let mut received: Received = unsafe { std::mem::zeroed() };
            // SAFETY: receives into a buffer of `size_of::<Received>()`
            // bytes on the receive right we own.
            let kr = unsafe {
                mach_msg(
                    &raw mut received.message.header,
                    MACH_RCV_MSG | TRAILER_FORMAT_0 | trailer_elements(MACH_RCV_TRAILER_AUDIT),
                    0,
                    size_of::<Received>(),
                    port,
                    MACH_MSG_TIMEOUT_NONE,
                    MACH_PORT_NULL,
                )
            };
            if kr != MACH_MSG_SUCCESS {
                // Too large (destroyed by the kernel) or interrupted.
                continue;
            }
            self.arrived(&mut received);
        }
    }

    fn arrived(&self, received: &mut Received) {
        let m = received.message;
        // The sender's pid, stamped by the kernel (audit_token_to_pid).
        let pid = received.trailer.msgh_audit.val[5];
        let well_formed = m.header.msgh_id == MESSAGE_ID
            && m.header.msgh_bits & MACH_MSGH_BITS_COMPLEX != 0
            && m.header.msgh_size == size_of::<Message>()
            && m.body.msgh_descriptor_count == u32::from(MAX_SLOTS)
            && m.count <= u32::from(MAX_SLOTS)
            && m.ports.iter().all(|p| {
                u32::from(p.type_) == MACH_MSG_PORT_DESCRIPTOR || p.name == MACH_PORT_NULL
            });
        let mut arrivals = lock(&self.arrivals);
        if well_formed && arrivals.expected.contains(&pid) {
            let count = usize::try_from(m.count).unwrap_or(0);
            let surfaces: Option<Vec<IoSurface>> = m
                .ports
                .iter()
                .take(count)
                .map(|p| IOSurfaceRef::lookup_from_mach_port(p.name).map(IoSurface))
                .collect();
            if let Some(surfaces) = surfaces {
                let buffers = u64::from(m.buffers[0]) | (u64::from(m.buffers[1]) << 32);
                let sets = arrivals.sets.entry(pid).or_default();
                sets.push_back((buffers, surfaces));
                while sets.len() > KEPT {
                    sets.pop_front();
                }
            }
        }
        drop(arrivals);
        // The surfaces hold themselves now: release every right the
        // message brought (its ports, any reply port).
        // SAFETY: the message as received, each right in it ours.
        unsafe { mach_msg_destroy(&raw mut received.message.header) };
        self.arrived.notify_all();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn a_surface_is_the_size_asked_with_aligned_rows() {
        let s = IoSurface::new(333, 17).unwrap();
        assert_eq!((s.width(), s.height()), (333, 17));
        assert!(s.stride() >= 333 * 4);
        assert_eq!(s.stride() % 16, 0, "rows aligned: {}", s.stride());
        assert!(IoSurface::new(0, 4).is_err());
    }

    /// The whole trip inside one process: a sender found by name, the
    /// surfaces taken by id, the same memory on both sides; and nothing
    /// kept from a sender the rendezvous was not told to expect.
    #[test]
    fn surfaces_reach_the_rendezvous_from_the_expected_sender_only() {
        let rendezvous = Rendezvous::get().expect("a rendezvous (not sandboxed)");
        let me = std::process::id();
        // SAFETY: a test's own environment, set before the lookup reads it.
        unsafe { std::env::set_var(ENV, rendezvous.name()) };
        let sender = Sender::from_env().unwrap().expect("the name set above");
        let a = IoSurface::new(64, 32).unwrap();
        let b = IoSurface::new(64, 32).unwrap();
        rendezvous.forget(me);
        sender.send(7, &[a.clone(), b.clone()]).unwrap();
        assert!(
            rendezvous.take(me, 7, Duration::from_millis(300)).is_none(),
            "not expected: dropped"
        );
        rendezvous.expect(me);
        sender.send(8, std::slice::from_ref(&a)).unwrap();
        sender.send(9, &[a.clone(), b.clone()]).unwrap();
        let got = rendezvous.take(me, 9, Duration::from_secs(5)).unwrap();
        assert_eq!(
            got.iter().map(IoSurface::id).collect::<Vec<_>>(),
            vec![a.id(), b.id()]
        );
        assert!(
            rendezvous.take(me, 8, Duration::ZERO).is_none(),
            "an older set is dropped once a newer one is taken"
        );
        rendezvous.forget(me);
    }
}
