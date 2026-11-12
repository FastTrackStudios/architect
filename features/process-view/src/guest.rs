//! The program's side (feature `guest`): a [`PaintView`] served from a
//! process of its own, into a pane of the app's window.
//!
//! [`serve`] is the whole of it. It takes the socket the app handed the
//! program ([`crate::sock::FD_ENV`]), waits for the pane's size, makes a
//! wgpu device on the app's adapter, and from then on records the view
//! with Vello into the frames it shares: each frame a dma-buf texture
//! the app's GPU reads in place ([`Transport::Dmabuf`]), or, where the
//! app can't import one, shared memory it uploads
//! ([`Transport::Shm`]). The view is the same trait a linked view
//! implements, so one view runs linked or in a process unchanged.
//!
//! Frames are paced by the app: the program draws only into a slot the
//! app released, so it draws as often as the app shows frames, and not
//! at all while nothing changes (a still view costs nothing).

use std::os::fd::{AsFd as _, OwnedFd};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList};
use architect_workspace::paint::PaintScene as _;
use architect_workspace::{
    FrameClock, Geometry, KindId, PaintView, Painter, PaneId, Placement, ViewCtx, ViewHost,
};
use vello::peniko::kurbo::Affine;

use crate::gpu::{self, Exported};
use crate::sock::Socket;
use crate::wire::{self, Adapter, ToGuest, ToHost, Transport};

/// Forces a transport: `shm` (or `dmabuf`, which falls back to `shm`
/// when it can't).
pub const TRANSPORT_ENV: &str = "ARCHITECT_PROCESS_VIEW_TRANSPORT";

/// Frames in flight: the one the app shows, the one before it (its GPU
/// may still read it), and two to draw into.
const SLOTS: u8 = 4;

/// What stopped a guest.
#[derive(Debug, thiserror::Error)]
pub enum GuestError {
    #[error("socket: {0}")]
    Io(#[from] std::io::Error),
    #[error("message: {0}")]
    Wire(#[from] wire::WireError),
    #[error("the app speaks protocol {0}, this program {VERSION}", VERSION = wire::VERSION)]
    Version(u16),
    #[error("no GPU adapter")]
    NoAdapter,
    #[error("GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("Vello: {0}")]
    Vello(#[from] vello::Error),
    #[error("GPU: {0}")]
    Gpu(#[from] gpu::GpuError),
    #[error("the app closed the view before saying hello")]
    NoHello,
}

/// Serves the view `make` returns, on the socket the app handed this
/// program, until the app closes it. The view is made once the pane's
/// first size is known.
pub fn serve(make: impl FnOnce(&ViewCtx) -> Box<dyn PaintView>) -> Result<(), GuestError> {
    let socket = Socket::from_env()?;
    let hello = loop {
        let Some((bytes, _)) = socket.recv()? else {
            return Err(GuestError::NoHello);
        };
        if let ToGuest::Hello { .. } = ToGuest::decode(&bytes)? {
            break ToGuest::decode(&bytes)?;
        }
    };
    let ToGuest::Hello {
        version,
        width,
        height,
        scale,
        adapter,
        dmabuf,
        theme,
    } = hello
    else {
        return Err(GuestError::NoHello);
    };
    if version != wire::VERSION {
        return Err(GuestError::Version(version));
    }
    let (device, queue) = pollster::block_on(open_device(&adapter))?;
    let forced_shm = std::env::var(TRANSPORT_ENV).is_ok_and(|v| v == "shm");
    let transport = if dmabuf && !forced_shm && gpu::can_share(&device) {
        Transport::Dmabuf
    } else {
        Transport::Shm
    };
    let began = Instant::now();
    let clock = FrameClock::new(Duration::from_millis(16), move || began.elapsed());
    let ctx = ViewCtx::new(
        PaneId::new("process-view"),
        KindId::new(crate::PROCESS),
        (ViewHost::Blitz, Placement::Pane),
        (ActionList::default(), ActionCaller::app()),
        clock,
    )
    .with_theme(theme);
    let view = make(&ctx);
    let mut guest = Guest {
        renderer: vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                ..vello::RendererOptions::default()
            },
        )?,
        painter: Painter::boxed(ctx, view),
        frames: Frames::new(&device, transport, (width.max(1), height.max(1)), 1)?,
        device,
        queue,
        socket,
        scale,
        seq: 0,
        input: 0,
        expose: true,
        credit: true,
    };
    guest.announce()?;
    guest.run()
}

/// The device on the app's adapter (by vendor and device id), else the
/// first that has one.
async fn open_device(want: &Adapter) -> Result<(wgpu::Device, wgpu::Queue), GuestError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = instance.enumerate_adapters(wgpu::Backends::VULKAN).await;
    let adapter = adapters
        .iter()
        .find(|a| {
            let info = a.get_info();
            info.vendor == want.vendor && info.device == want.device
        })
        .or_else(|| adapters.first())
        .ok_or(GuestError::NoAdapter)?;
    let features = adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("process view"),
            required_features: features,
            required_limits: adapter.limits(),
            ..wgpu::DeviceDescriptor::default()
        })
        .await?;
    Ok((device, queue))
}

/// The frames' memory: the slots, which of them the app holds, and the
/// target Vello renders into before a frame is copied into its slot.
struct Frames {
    id: u64,
    width: u32,
    height: u32,
    /// Vello's target (a storage texture).
    target: wgpu::Texture,
    shared: Shared,
    /// Whether the app holds each slot.
    held: Vec<bool>,
}

enum Shared {
    Dmabuf(Vec<Exported>),
    Shm {
        file: OwnedFd,
        map: Map,
        /// Where a frame is read back to, rows padded to 256 bytes.
        readback: wgpu::Buffer,
        padded: u32,
    },
}

/// A shared mapping, unmapped when dropped.
struct Map {
    ptr: *mut std::ffi::c_void,
    len: usize,
}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: mapped by `Map::new` with this length, used by nothing
        // else once dropped.
        let _ = unsafe { rustix::mm::munmap(self.ptr, self.len) };
    }
}

impl Frames {
    fn new(
        device: &wgpu::Device,
        transport: Transport,
        (width, height): (u32, u32),
        id: u64,
    ) -> Result<Self, GuestError> {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("process view target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gpu::FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let shared = match transport {
            Transport::Dmabuf => Shared::Dmabuf(
                (0..SLOTS)
                    .map(|_| gpu::export_texture(device, width, height))
                    .collect::<Result<_, _>>()?,
            ),
            Transport::Shm => {
                let row = width.checked_mul(4).ok_or(gpu::GpuError::Range)?;
                let padded = row
                    .checked_next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                    .ok_or(gpu::GpuError::Range)?;
                let len = slot_len(row, height)?
                    .checked_mul(usize::from(SLOTS))
                    .ok_or(gpu::GpuError::Range)?;
                let file = rustix::fs::memfd_create(
                    "architect-process-view",
                    rustix::fs::MemfdFlags::CLOEXEC,
                )
                .map_err(std::io::Error::from)?;
                rustix::fs::ftruncate(&file, u64::try_from(len).map_err(|_| gpu::GpuError::Range)?)
                    .map_err(std::io::Error::from)?;
                // SAFETY: a fresh mapping of a file we own, `len` long.
                let ptr = unsafe {
                    rustix::mm::mmap(
                        std::ptr::null_mut(),
                        len,
                        rustix::mm::ProtFlags::READ | rustix::mm::ProtFlags::WRITE,
                        rustix::mm::MapFlags::SHARED,
                        &file,
                        0,
                    )
                }
                .map_err(std::io::Error::from)?;
                let readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("process view readback"),
                    size: u64::from(padded)
                        .checked_mul(u64::from(height))
                        .ok_or(gpu::GpuError::Range)?,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                Shared::Shm {
                    file,
                    map: Map { ptr, len },
                    readback,
                    padded,
                }
            }
        };
        Ok(Self {
            id,
            width,
            height,
            target,
            shared,
            held: vec![false; usize::from(SLOTS)],
        })
    }

    /// The message announcing these buffers.
    fn message(&self) -> ToHost {
        match &self.shared {
            Shared::Dmabuf(slots) => {
                let first = slots.first();
                ToHost::Buffers {
                    id: self.id,
                    transport: Transport::Dmabuf,
                    width: self.width,
                    height: self.height,
                    slots: SLOTS,
                    stride: first.map_or(0, |e| e.stride),
                    offset: first.map_or(0, |e| e.offset),
                    modifier: first.map_or(gpu::LINEAR, |e| e.modifier),
                }
            }
            Shared::Shm { .. } => ToHost::Buffers {
                id: self.id,
                transport: Transport::Shm,
                width: self.width,
                height: self.height,
                slots: SLOTS,
                stride: self.width.saturating_mul(4),
                offset: 0,
                modifier: gpu::LINEAR,
            },
        }
    }

    fn free_slot(&self) -> Option<u8> {
        self.held
            .iter()
            .position(|held| !held)
            .and_then(|i| u8::try_from(i).ok())
    }
}

/// One slot's bytes in shared memory.
fn slot_len(row: u32, height: u32) -> Result<usize, gpu::GpuError> {
    usize::try_from(u64::from(row).saturating_mul(u64::from(height)))
        .map_err(|_| gpu::GpuError::Range)
}

struct Guest {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    painter: Painter,
    frames: Frames,
    socket: Socket,
    scale: f32,
    seq: u64,
    /// The last input event given to the view.
    input: u64,
    /// Draw one now, whatever changed.
    expose: bool,
    /// Whether the app is ready for a frame (it showed the last one).
    credit: bool,
}

impl Guest {
    fn announce(&self) -> Result<(), GuestError> {
        let bytes = self.frames.message().encode()?;
        match &self.frames.shared {
            Shared::Dmabuf(slots) => {
                let fds: Vec<_> = slots.iter().map(|e| e.fd.as_fd()).collect();
                self.socket.send(&bytes, &fds)?;
            }
            Shared::Shm { file, .. } => self.socket.send(&bytes, &[file.as_fd()])?,
        }
        Ok(())
    }

    fn run(&mut self) -> Result<(), GuestError> {
        loop {
            let want = self.expose || self.painter.wants_frame();
            let free = self.frames.free_slot().filter(|_| self.credit);
            // Drawing now: take what is waiting first. Otherwise sleep
            // until the app says something (input, a release).
            let timeout = (want && free.is_some()).then_some(Duration::ZERO);
            if self.socket.wait(timeout)? {
                let Some((bytes, _)) = self.socket.recv()? else {
                    return Ok(());
                };
                self.handle(ToGuest::decode(&bytes)?)?;
                continue;
            }
            if let (true, Some(slot)) = (want, free) {
                self.draw(slot)?;
            }
        }
    }

    fn handle(&mut self, message: ToGuest) -> Result<(), GuestError> {
        match message {
            ToGuest::Input { seq, input } => {
                self.painter.ctx().input.push(input);
                self.input = seq;
            }
            ToGuest::Release { buffers, slot } => {
                if buffers == self.frames.id
                    && let Some(held) = self.frames.held.get_mut(usize::from(slot))
                {
                    *held = false;
                }
            }
            ToGuest::Resize {
                width,
                height,
                scale,
            } => {
                self.scale = scale;
                let size = (width.max(1), height.max(1));
                if size != (self.frames.width, self.frames.height) {
                    let transport = match self.frames.shared {
                        Shared::Dmabuf(_) => Transport::Dmabuf,
                        Shared::Shm { .. } => Transport::Shm,
                    };
                    let id = self.frames.id.saturating_add(1);
                    self.frames = Frames::new(&self.device, transport, size, id)?;
                    self.announce()?;
                }
                self.expose = true;
                self.credit = true;
            }
            ToGuest::Expose => {
                self.expose = true;
                self.credit = true;
            }
            ToGuest::Tick => self.credit = true,
            ToGuest::Theme(theme) => self.painter.set_theme(theme),
            // A second hello: the app could not import the dma-bufs (it
            // asks for shared memory), or a new pane for the same program.
            ToGuest::Hello { dmabuf, .. } => {
                if !dmabuf && matches!(self.frames.shared, Shared::Dmabuf(_)) {
                    let size = (self.frames.width, self.frames.height);
                    let id = self.frames.id.saturating_add(1);
                    self.frames = Frames::new(&self.device, Transport::Shm, size, id)?;
                }
                self.expose = true;
                self.credit = true;
                self.announce()?;
            }
        }
        Ok(())
    }

    #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
    fn draw(&mut self, slot: u8) -> Result<(), GuestError> {
        let began = Instant::now();
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let (width, height) = (self.frames.width, self.frames.height);
        let scene = self.painter.frame(Geometry {
            width: width as f32 / scale,
            height: height as f32 / scale,
            scale,
        });
        self.expose = false;
        self.credit = false;
        let mut vello_scene = vello::Scene::new();
        anyrender_vello::VelloScenePainter::new(&mut vello_scene)
            .append_scene(scene, Affine::IDENTITY);
        let view = self
            .frames
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.render_to_texture(
            &self.device,
            &self.queue,
            &vello_scene,
            &view,
            &vello::RenderParams {
                base_color: vello::peniko::Color::from_rgba8(0, 0, 0, 255),
                width,
                height,
                antialiasing_method: vello::AaConfig::Area,
            },
        )?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        match &self.frames.shared {
            Shared::Dmabuf(slots) => {
                if let Some(to) = slots.get(usize::from(slot)) {
                    encoder.copy_texture_to_texture(
                        self.frames.target.as_image_copy(),
                        to.texture.as_image_copy(),
                        size,
                    );
                }
            }
            Shared::Shm {
                readback, padded, ..
            } => encoder.copy_texture_to_buffer(
                self.frames.target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(*padded),
                        rows_per_image: Some(height),
                    },
                },
                size,
            ),
        }
        let submitted = self.queue.submit([encoder.finish()]);
        // The frame's pixels are ready before the app is told: the
        // CPU-side fence the module docs describe.
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submitted),
            timeout: Some(Duration::from_secs(2)),
        });
        if let Shared::Shm {
            map,
            readback,
            padded,
            ..
        } = &self.frames.shared
        {
            copy_out(&self.device, readback, map, (*padded, width, height), slot)?;
        }
        if let Some(held) = self.frames.held.get_mut(usize::from(slot)) {
            *held = true;
        }
        self.seq = self.seq.saturating_add(1);
        let render_us = u32::try_from(began.elapsed().as_micros()).unwrap_or(u32::MAX);
        let frame = ToHost::Frame {
            buffers: self.frames.id,
            slot,
            seq: self.seq,
            input: self.input,
            render_us,
        };
        self.socket.send(&frame.encode()?, &[])?;
        Ok(())
    }
}

/// The read-back frame into `slot` of the shared memory, its rows'
/// padding dropped.
fn copy_out(
    device: &wgpu::Device,
    readback: &wgpu::Buffer,
    map: &Map,
    (padded, width, height): (u32, u32, u32),
    slot: u8,
) -> Result<(), GuestError> {
    let range = readback.slice(..);
    range.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(2)),
    });
    let row = usize::try_from(width.saturating_mul(4)).map_err(|_| gpu::GpuError::Range)?;
    let padded = usize::try_from(padded).map_err(|_| gpu::GpuError::Range)?;
    let len = slot_len(width.saturating_mul(4), height)?;
    let start = len
        .checked_mul(usize::from(slot))
        .ok_or(gpu::GpuError::Range)?;
    if start.checked_add(len).is_none_or(|end| end > map.len) {
        return Err(gpu::GpuError::Range.into());
    }
    {
        let Ok(data) = range.get_mapped_range() else {
            readback.unmap();
            return Ok(());
        };
        // SAFETY: `start..start + len` is inside the mapping (checked
        // above); the app reads this slot only after the frame message.
        let out = unsafe { std::slice::from_raw_parts_mut(map.ptr.cast::<u8>().add(start), len) };
        for (to, from) in out.chunks_exact_mut(row).zip(data.chunks(padded)) {
            if let Some(from) = from.get(..row) {
                to.copy_from_slice(from);
            }
        }
    }
    readback.unmap();
    Ok(())
}
