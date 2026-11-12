//! The `process-view-demo` program serving frames to a host with a
//! wgpu device of its own and no window: over dma-buf textures (where
//! the GPU can share them) and over shared memory, each frame's pixels
//! read back from the host's own device; and the socket closing when the
//! program is killed.
//!
//! Needs a Vulkan adapter. Without one (a CI runner with no GPU) each
//! test says so and passes.

#![cfg(all(
    target_os = "linux",
    feature = "guest",
    feature = "host",
    feature = "demo"
))]

use std::os::fd::{AsFd as _, AsRawFd as _, OwnedFd};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use architect_process_view::gpu;
use architect_process_view::sock::{FD_ENV, Socket};
use architect_process_view::wire::{self, ToGuest, ToHost, Transport};
use architect_workspace::{ThemeTokens, ViewInput};

const W: u32 = 320;
const H: u32 = 200;

fn device() -> Option<(wgpu::Device, wgpu::Queue, wgpu::Adapter)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .ok()?;
        let features = adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
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
    cmd.env(FD_ENV, guest.as_raw_fd().to_string());
    if let Some(t) = transport {
        cmd.env("ARCHITECT_PROCESS_VIEW_TRANSPORT", t);
    }
    let child = cmd.spawn().unwrap();
    drop(guest);
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
    let mut encoder = device.create_command_encoder(&Default::default());
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
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
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

fn hello(socket: &Socket, adapter: &wgpu::Adapter, dmabuf: bool) {
    let hello = ToGuest::Hello {
        version: wire::VERSION,
        width: W,
        height: H,
        scale: 1.0,
        adapter: gpu::adapter_info(adapter),
        dmabuf,
        theme: ThemeTokens::default(),
    };
    socket.send(&hello.encode().unwrap(), &[]).unwrap();
}

#[test]
fn frames_arrive_as_dma_bufs_the_host_device_reads_in_place() {
    let Some((device, queue, adapter)) = device() else {
        eprintln!("no Vulkan adapter: skipped");
        return;
    };
    if !gpu::can_share(&device) {
        eprintln!("{}: no dma-buf sharing: skipped", adapter.get_info().name);
        return;
    }
    let (mut child, socket) = start(None);
    hello(&socket, &adapter, true);
    let (buffers, fds) = recv(&socket);
    let ToHost::Buffers {
        id,
        transport,
        width,
        height,
        slots,
        stride,
        offset,
        modifier,
    } = buffers
    else {
        panic!("buffers first, got {buffers:?}");
    };
    assert_eq!((transport, width, height), (Transport::Dmabuf, W, H));
    assert_eq!(fds.len(), usize::from(slots));
    let textures: Vec<_> = fds
        .into_iter()
        .map(|fd| gpu::import_dmabuf(&device, fd, (W, H), (stride, offset, modifier)).unwrap())
        .collect();
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
    let shown = loop {
        let (frame, _) = recv(&socket);
        if let ToHost::Frame { input: 1, slot, .. } = frame {
            break slot;
        }
        if let ToHost::Frame { buffers, slot, .. } = frame {
            let release = ToGuest::Release { buffers, slot };
            socket.send(&release.encode().unwrap(), &[]).unwrap();
            socket.send(&ToGuest::Tick.encode().unwrap(), &[]).unwrap();
        }
    };
    eprintln!(
        "dma-buf on {}: input to frame {:?}",
        adapter.get_info().name,
        sent.elapsed()
    );
    assert_drawn(&read_back(&device, &queue, &textures[usize::from(shown)]));
    child.kill().unwrap();
    child.wait().unwrap();
    // The program's end is closed: the host reads the end, not a hang.
    assert!(socket.wait(Some(Duration::from_secs(5))).unwrap());
    while let Some(_) = socket.recv().unwrap() {}
}

#[test]
fn frames_arrive_in_shared_memory_where_dma_bufs_cannot() {
    let Some((device, _queue, adapter)) = device() else {
        eprintln!("no Vulkan adapter: skipped");
        return;
    };
    let (mut child, socket) = start(Some("shm"));
    hello(&socket, &adapter, gpu::can_share(&device));
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
    let file = std::fs::File::from(fds.into_iter().next().unwrap());
    let len = (stride * H) as usize * usize::from(slots);
    assert_eq!(file.metadata().unwrap().len() as usize, len);
    let mut all = vec![0u8; len];
    rustix::io::pread(file.as_fd(), &mut all, 0).unwrap();
    let start = (stride * H) as usize * usize::from(slot);
    assert_drawn(&all[start..start + (stride * H) as usize]);
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
