//! The `process-view-demo` program serving frames to a host with a
//! wgpu device of its own and no window: over textures the GPU shares
//! (dma-bufs on Linux, `IOSurface`s on macOS) and over shared memory,
//! each frame's pixels read back from the host's own device; and the
//! socket closing when the program is killed.
//!
//! Needs a Vulkan adapter (Linux) or a Metal one (macOS). Without one (a
//! CI runner with no GPU) each test says so and passes.

// Pixel rows and buffer sizes are small, checked by the test itself.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]
#![cfg(all(
    any(target_os = "linux", target_os = "macos"),
    feature = "guest",
    feature = "host",
    feature = "demo"
))]

use std::os::fd::OwnedFd;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use architect_process_view::gpu;
use architect_process_view::sock::{FD_ENV, Socket};
use architect_process_view::wire::{self, ToGuest, ToHost, Transport};
use architect_workspace::{ThemeTokens, ViewInput};

/// The GPU tests take turns: two Vulkan devices made and torn down at once
/// in one process (one of them with imported dma-bufs) stalled the NVIDIA
/// driver here, a driver thread spinning a core for as long as the run.
fn gpu_alone() -> std::sync::MutexGuard<'static, ()> {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ONE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

thread_local! {
    static STEP: std::cell::RefCell<Option<std::sync::Arc<std::sync::Mutex<&'static str>>>> =
        const { std::cell::RefCell::new(None) };
}

/// What this test is doing now, for its watchdog.
fn step(what: &'static str) {
    STEP.with(|s| {
        if let Some(at) = &*s.borrow() {
            *at.lock().unwrap() = what;
        }
    });
}

/// Watches the test on this thread: if it has not finished within
/// `within`, the run ends (abort) saying the step it was stuck at, rather
/// than a stalled call (a GPU driver's) holding a core and the run for
/// ever. Dropped when the test returns.
struct Watchdog(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Watchdog {
    fn new(test: &'static str, within: Duration) -> Self {
        let at = std::sync::Arc::new(std::sync::Mutex::new("starting"));
        STEP.with(|s| *s.borrow_mut() = Some(at.clone()));
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = done.clone();
        std::thread::spawn(move || {
            let until = Instant::now() + within;
            while Instant::now() < until {
                if seen.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            eprintln!(
                "{test}: not finished within {within:?}; stuck at: {}",
                at.lock().unwrap()
            );
            std::process::abort();
        });
        Self(done)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

const W: u32 = 320;
const H: u32 = 200;

fn device() -> Option<(wgpu::Device, wgpu::Queue, wgpu::Adapter)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: gpu::BACKENDS,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .ok()?;
        let features = gpu::sharing_features(&adapter);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: features,
                ..Default::default()
            })
            .await
            .ok()?;
        Some((device, queue, adapter))
    })
}

fn start(transport: Option<&str>) -> (Child, Socket) {
    let (host, guest) = Socket::pair().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_process-view-demo"));
    architect_process_view::sock::hand_to(&mut cmd, &guest);
    if let Some(t) = transport {
        cmd.env("ARCHITECT_PROCESS_VIEW_TRANSPORT", t);
    }
    let child = cmd.spawn().unwrap();
    drop(guest);
    // macOS: its surfaces are taken (as `host::Session` does at spawn).
    #[cfg(target_os = "macos")]
    if let Some(r) = architect_process_view::surface::Rendezvous::get() {
        r.expect(child.id());
    }
    (child, host)
}

fn recv(socket: &Socket) -> (ToHost, Vec<OwnedFd>) {
    assert!(
        socket.wait(Some(Duration::from_secs(20))).unwrap(),
        "no message in 20 s"
    );
    let (bytes, fds) = socket
        .recv()
        .unwrap()
        .expect("the program closed the socket");
    (ToHost::decode(&bytes).unwrap(), fds)
}

/// The pixels of `texture` (RGBA8, `W` × `H`) on the host's device.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let padded = (W * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded * H),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    step("read back: submitting the copy");
    queue.submit([encoder.finish()]);
    step("read back: mapping the buffer");
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    step("read back: waiting for the GPU (poll)");
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })
        .unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap();
    data.chunks(padded as usize)
        .flat_map(|row| row[..(W * 4) as usize].to_vec())
        .collect()
}

/// What a frame of the demo looks like: its background, a lit sweep,
/// and particles (so neither all one colour nor empty).
fn assert_drawn(pixels: &[u8]) {
    assert_eq!(pixels.len(), (W * H * 4) as usize);
    let background = [14u8, 14, 20, 255];
    let bg = pixels.chunks(4).filter(|p| *p == background).count();
    let other = pixels
        .chunks(4)
        .filter(|p| *p != background && p[3] == 255)
        .count();
    assert!(
        bg > (W * H / 2) as usize,
        "mostly background: {bg} of {}",
        W * H
    );
    assert!(other > 200, "particles and the sweep drawn: {other} pixels");
}

fn hello(socket: &Socket, adapter: &wgpu::Adapter, zero_copy: bool) {
    let hello = ToGuest::Hello {
        version: wire::VERSION,
        width: W,
        height: H,
        scale: 1.0,
        adapter: gpu::adapter_info(adapter),
        zero_copy,
        theme: ThemeTokens::default(),
    };
    socket.send(&hello.encode().unwrap(), &[]).unwrap();
}

/// The slots' textures of the host's device, from what came beside the
/// buffers' message: each dma-buf imported in place.
#[cfg(target_os = "linux")]
fn import_slots(
    device: &wgpu::Device,
    _child: &Child,
    buffers: &ToHost,
    fds: Vec<OwnedFd>,
) -> Vec<wgpu::Texture> {
    let ToHost::Buffers {
        transport,
        slots,
        stride,
        offset,
        modifier,
        ..
    } = *buffers
    else {
        panic!("buffers first, got {buffers:?}");
    };
    assert_eq!(transport, Transport::Dmabuf);
    assert_eq!(fds.len(), usize::from(slots));
    fds.into_iter()
        .map(|fd| gpu::import_dmabuf(device, fd, (W, H), (stride, offset, modifier)).unwrap())
        .collect()
}

/// The slots' textures of the host's device: each `IOSurface` the
/// program sent this process's rendezvous for these buffers, a texture
/// over the same memory.
#[cfg(target_os = "macos")]
fn import_slots(
    device: &wgpu::Device,
    child: &Child,
    buffers: &ToHost,
    fds: Vec<OwnedFd>,
) -> Vec<wgpu::Texture> {
    let ToHost::Buffers {
        id,
        transport,
        slots,
        stride,
        ..
    } = *buffers
    else {
        panic!("buffers first, got {buffers:?}");
    };
    assert_eq!(transport, Transport::IoSurface);
    assert!(fds.is_empty(), "surfaces travel on the Mach port");
    drop(fds);
    let surfaces = architect_process_view::surface::Rendezvous::get()
        .unwrap()
        .take(child.id(), id, Duration::from_secs(5))
        .expect("the program's surfaces for these buffers");
    assert_eq!(surfaces.len(), usize::from(slots));
    assert_eq!(surfaces[0].stride(), stride);
    surfaces
        .iter()
        .map(|s| gpu::import_surface(device, s).unwrap())
        .collect()
}

#[test]
fn frames_arrive_as_textures_the_host_device_reads_in_place() {
    let _alone = gpu_alone();
    // Declared first: dropped last, after the device and its textures.
    let _watch = Watchdog::new("GPU-shared frames", Duration::from_secs(60));
    step("making a device");
    let Some((device, queue, adapter)) = device() else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    if !gpu::can_share(&device) {
        eprintln!("{}: no texture sharing: skipped", adapter.get_info().name);
        return;
    }
    let (mut child, socket) = start(None);
    hello(&socket, &adapter, true);
    step("waiting for its buffers");
    let (buffers, fds) = recv(&socket);
    let ToHost::Buffers {
        id, width, height, ..
    } = buffers
    else {
        panic!("buffers first, got {buffers:?}");
    };
    assert_eq!((width, height), (W, H));
    step("importing its slots");
    let textures = import_slots(&device, &child, &buffers, fds);
    let slots = u8::try_from(textures.len()).unwrap();
    // A frame, then a press; the frame that shows the press says so.
    let (frame, _) = recv(&socket);
    let ToHost::Frame { buffers, slot, .. } = frame else {
        panic!("a frame, got {frame:?}");
    };
    assert_eq!(buffers, id);
    assert_drawn(&read_back(&device, &queue, &textures[usize::from(slot)]));
    let sent = Instant::now();
    let press = ToGuest::Input {
        seq: 1,
        input: ViewInput::PointerDown {
            x: 100.0,
            y: 100.0,
            button: architect_workspace::Button::Primary,
            mods: Vec::new(),
        },
    };
    socket.send(&press.encode().unwrap(), &[]).unwrap();
    for s in 0..slots {
        let release = ToGuest::Release {
            buffers: id,
            slot: s,
        };
        socket.send(&release.encode().unwrap(), &[]).unwrap();
    }
    socket.send(&ToGuest::Tick.encode().unwrap(), &[]).unwrap();
    // The program answers each tick with a frame: without a deadline a
    // frame that never shows the press would ping-pong for ever.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    let shown = loop {
        assert!(
            Instant::now() < deadline,
            "no frame showing the press (input 1) within 30 s; the frames' inputs: {seen:?}"
        );
        let (frame, _) = recv(&socket);
        if let ToHost::Frame { input: 1, slot, .. } = frame {
            break slot;
        }
        if let ToHost::Frame { input, .. } = frame
            && seen.len() < 64
        {
            seen.push(input);
        }
        if let ToHost::Frame { buffers, slot, .. } = frame {
            let release = ToGuest::Release { buffers, slot };
            socket.send(&release.encode().unwrap(), &[]).unwrap();
            socket.send(&ToGuest::Tick.encode().unwrap(), &[]).unwrap();
        }
    };
    eprintln!(
        "GPU-shared frames on {}: input to frame {:?}",
        adapter.get_info().name,
        sent.elapsed()
    );
    step("reading the frame back");
    assert_drawn(&read_back(&device, &queue, &textures[usize::from(shown)]));
    step("killing the program");
    child.kill().unwrap();
    child.wait().unwrap();
    step("reading the killed program's socket to its end");
    // The program's end is closed: the host reads the end, not a hang.
    // Its last frames, then the end: each wait bounded.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            socket.wait(Some(Duration::from_secs(5))).unwrap(),
            "the killed program's socket neither closed nor spoke within 5 s"
        );
        if socket.recv().unwrap().is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the killed program's socket still sending after 10 s"
        );
    }
    step("dropping the imported textures and the device (the GPU driver)");
}

#[test]
fn frames_arrive_in_shared_memory_where_dma_bufs_cannot() {
    let _alone = gpu_alone();
    let _watch = Watchdog::new("shared-memory frames", Duration::from_secs(60));
    step("making a device");
    let Some((device, _queue, adapter)) = device() else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    step("starting the program (shared memory)");
    let (mut child, socket) = start(Some("shm"));
    hello(&socket, &adapter, gpu::can_share(&device));
    step("waiting for its buffers");
    let (buffers, fds) = recv(&socket);
    let ToHost::Buffers {
        transport,
        stride,
        slots,
        ..
    } = buffers
    else {
        panic!("buffers first, got {buffers:?}");
    };
    assert_eq!(transport, Transport::Shm);
    assert_eq!(fds.len(), 1);
    let (frame, _) = recv(&socket);
    let ToHost::Frame { slot, .. } = frame else {
        panic!("a frame, got {frame:?}");
    };
    // At least as long as announced (macOS rounds shared memory up to a
    // page), read as the app does: mapped.
    let len = (stride * H) as usize * usize::from(slots);
    let map = architect_process_view::shm::Shared::map(fds.into_iter().next().unwrap(), len, false)
        .unwrap();
    let start = (stride * H) as usize * usize::from(slot);
    // SAFETY: the program doesn't write a slot it sent until it is released.
    assert_drawn(unsafe { map.bytes(start, (stride * H) as usize) }.unwrap());
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn a_program_that_is_not_a_view_program_is_an_error_not_a_hang() {
    // The program, run without its socket, says so and exits.
    let out = Command::new(env!("CARGO_BIN_EXE_process-view-demo"))
        .env_remove(FD_ENV)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains(FD_ENV));
}
