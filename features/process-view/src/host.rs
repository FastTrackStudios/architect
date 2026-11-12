//! The app's side (feature `host`): a process view in a pane.
//!
//! - [`Session`] is one running view program: started with a socket
//!   pair, read by a thread of its own, its liveness ([`Status`]) and its
//!   latest frames kept for whichever widget shows it. Sessions live in
//!   [`sessions`] by key (a pane id), not in a component, so a pane that
//!   moves (a new tab group, a split, another window) shows the same
//!   program without restarting it.
//! - [`ProcessViewWidget`] is the Blitz custom widget: it says hello with
//!   the pane's size and the window's GPU adapter, imports the program's
//!   frames into the window's own wgpu device (dma-bufs in place, or
//!   uploads from shared memory), draws the newest, forwards the pane's
//!   input and resizes, and releases each frame when it no longer needs
//!   it.
//! - [`ProcessViewPane`] is the pane around it: the widget, a line of
//!   numbers, and the stopped state with a button that starts the
//!   program again.
//!
//! The app never waits for the program: every send is a datagram on a
//! socket, every receive happens on the session's thread. A program that
//! hangs leaves its last frame up with "not responding"; one that dies
//! leaves "view stopped". Neither touches the window beyond its pane's
//! rectangle, and it is handed input only while the pane has it.

use std::collections::{HashMap, VecDeque};
use std::os::fd::{AsFd as _, AsRawFd as _, OwnedFd};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use anyrender::{PaintScene as _, RenderContext, ResourceId, Scene};
use architect_workspace::paint::view_input;
use architect_workspace::{FrameStats, FrameSummary, ThemeTokens, ViewInput};
use blitz_dom::node::{ComputedStyles, Widget};
use blitz_traits::events::UiEvent;
use blitz_traits::shell::ShellProvider;
use dioxus::prelude::*;
use dioxus_native_dom::CustomWidgetAttr;
use peniko::kurbo::{Affine, Rect};
use peniko::{Color, Fill, ImageBrush, ImageSampler};

use crate::gpu;
use crate::sock::{FD_ENV, Socket};
use crate::wire::{self, ToGuest, ToHost, Transport};

/// A view program: what to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub path: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Program {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }
}

/// A view program's liveness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Started, no frame yet.
    Starting,
    Running,
    /// Gone, and why (its exit, a signal, a broken message).
    Stopped(String),
}

/// The last frame's buffers the program announced, with their
/// descriptors (kept so a new widget can import them again).
struct Announced {
    message: ToHost,
    fds: Vec<OwnedFd>,
}

#[derive(Debug, Clone, Copy)]
struct Arrived {
    buffers: u64,
    slot: u8,
    /// When the input event this frame shows was sent.
    sent: Option<Instant>,
}

struct State {
    status: Status,
    /// Starts so far: a widget that saw an older one starts over.
    generation: u64,
    socket: Option<Arc<Socket>>,
    child: Option<Child>,
    pid: Option<u32>,
    buffers: Option<Announced>,
    frames: VecDeque<Arrived>,
    /// The generation the program was said hello to.
    hello: Option<u64>,
    /// The last size it was told.
    size: Option<(u32, u32, f32)>,
    theme: ThemeTokens,
    /// When each input event was sent, by number, until a frame shows it.
    sent: VecDeque<(u64, Instant)>,
    input_seq: u64,
    /// When the oldest input with no frame since was sent.
    unanswered: Option<Instant>,
    transport: Option<Transport>,
    frames_shown: u64,
    /// The window's GPU adapter, as told to the program.
    adapter: Option<String>,
}

/// What a session measured.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    /// The program's frame times: recording, Vello, its GPU's wait.
    pub render: FrameStats,
    /// From an input event's send to the arrival of the frame that
    /// shows it.
    pub input_to_frame: FrameStats,
    /// From an input event's send to the paint that draws that frame.
    pub input_to_paint: FrameStats,
    /// The widget's own paint (imports, uploads, the scene).
    pub host_paint: FrameStats,
}

/// One view program and what it showed last.
pub struct Session {
    key: String,
    program: Program,
    state: Mutex<State>,
    window: Mutex<Option<Arc<dyn ShellProvider>>>,
    stats: Stats,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("key", &self.key)
            .field("program", &self.program)
            .finish_non_exhaustive()
    }
}

/// A snapshot of a session, for a status line or an action.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub status: Status,
    pub pid: Option<u32>,
    pub generation: u64,
    pub transport: Option<Transport>,
    pub size: Option<(u32, u32)>,
    pub frames: u64,
    pub render: FrameSummary,
    pub input_to_frame: FrameSummary,
    pub input_to_paint: FrameSummary,
    pub host_paint: FrameSummary,
    /// An input event has waited this long for a frame.
    pub waiting: Option<Duration>,
    /// The GPU adapter both sides draw on.
    pub adapter: Option<String>,
}

/// Past this, a program that hasn't answered input is "not responding".
pub const HANG: Duration = Duration::from_secs(1);

/// The sessions by key.
pub fn sessions() -> &'static Mutex<HashMap<String, Arc<Session>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<Session>>>> = OnceLock::new();
    SESSIONS.get_or_init(Mutex::default)
}

/// The session for `key`, started with `program` if there is none.
pub fn session(key: &str, program: &Program) -> Arc<Session> {
    let mut all = sessions().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(s) = all.get(key) {
        return s.clone();
    }
    let s = Session::start(key, program.clone());
    all.insert(key.to_owned(), s.clone());
    s
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Session {
    /// `program` started for `key`.
    #[must_use]
    pub fn start(key: &str, program: Program) -> Arc<Self> {
        let session = Arc::new(Self {
            key: key.to_owned(),
            program,
            state: Mutex::new(State {
                status: Status::Starting,
                generation: 0,
                socket: None,
                child: None,
                pid: None,
                buffers: None,
                frames: VecDeque::new(),
                hello: None,
                size: None,
                theme: ThemeTokens::default(),
                sent: VecDeque::new(),
                input_seq: 0,
                unanswered: None,
                transport: None,
                frames_shown: 0,
                adapter: None,
            }),
            window: Mutex::new(None),
            stats: Stats::default(),
        });
        session.restart();
        session
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub const fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Stops the program if it runs, and starts it again.
    pub fn restart(self: &Arc<Self>) {
        let mut state = lock(&self.state);
        if let Some(mut child) = state.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        state.generation = state.generation.saturating_add(1);
        state.socket = None;
        state.buffers = None;
        state.frames.clear();
        state.hello = None;
        state.size = None;
        state.sent.clear();
        state.unanswered = None;
        state.transport = None;
        state.pid = None;
        let generation = state.generation;
        match self.spawn() {
            Ok((child, socket)) => {
                state.status = Status::Starting;
                state.pid = Some(child.id());
                state.child = Some(child);
                state.socket = Some(socket.clone());
                drop(state);
                let session = self.clone();
                let _ = std::thread::Builder::new()
                    .name(format!("process-view {}", self.key))
                    .spawn(move || session.read(&socket, generation));
            }
            Err(e) => {
                state.status = Status::Stopped(format!(
                    "could not start {}: {e}",
                    self.program.path.display()
                ));
            }
        }
        self.wake();
    }

    /// Stops the program; the session stays, stopped.
    pub fn stop(&self) {
        let mut state = lock(&self.state);
        if let Some(mut child) = state.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        state.socket = None;
        state.status = Status::Stopped("stopped by the app".into());
    }

    fn spawn(&self) -> std::io::Result<(Child, Arc<Socket>)> {
        let (host, guest) = Socket::pair()?;
        let child = Command::new(&self.program.path)
            .args(&self.program.args)
            .envs(self.program.env.iter().map(|(k, v)| (k, v)))
            .env(FD_ENV, guest.as_raw_fd().to_string())
            .stdin(Stdio::null())
            .spawn()?;
        // Only the program holds its end now: its exit closes the socket.
        drop(guest);
        Ok((child, Arc::new(host)))
    }

    /// The session's thread: every message from the program until it
    /// closes its end.
    fn read(self: &Arc<Self>, socket: &Socket, generation: u64) {
        let reason = loop {
            match socket.recv() {
                Ok(Some((bytes, fds))) => match ToHost::decode(&bytes) {
                    Ok(message) => {
                        if !self.arrived(generation, message, fds) {
                            return;
                        }
                    }
                    Err(e) => break format!("a broken message: {e}"),
                },
                Ok(None) => break String::new(),
                Err(e) => break format!("its socket failed: {e}"),
            }
        };
        let mut state = lock(&self.state);
        if state.generation != generation {
            return;
        }
        let exit = state.child.as_mut().and_then(|c| wait_briefly(c));
        state.child = None;
        state.socket = None;
        state.status = Status::Stopped(match (exit, reason.is_empty()) {
            (Some(exit), true) => exit,
            (Some(exit), false) => format!("{reason}; {exit}"),
            (None, _) if reason.is_empty() => "it closed the view".into(),
            (None, _) => reason,
        });
        drop(state);
        self.wake();
    }

    /// One message from the program; false once this generation is over.
    fn arrived(&self, generation: u64, message: ToHost, fds: Vec<OwnedFd>) -> bool {
        let mut state = lock(&self.state);
        if state.generation != generation {
            return false;
        }
        match message {
            ToHost::Buffers { transport, .. } => {
                state.transport = Some(transport);
                state.frames.clear();
                state.buffers = Some(Announced { message, fds });
            }
            ToHost::Frame {
                buffers,
                slot,
                input,
                render_us,
                ..
            } => {
                state.status = Status::Running;
                self.stats
                    .render
                    .record(Duration::from_micros(u64::from(render_us)));
                let sent = sent_at(&mut state.sent, input);
                if let Some(at) = sent {
                    self.stats.input_to_frame.record(at.elapsed());
                }
                if input >= state.input_seq {
                    state.unanswered = None;
                }
                state.frames.push_back(Arrived {
                    buffers,
                    slot,
                    sent,
                });
            }
        }
        drop(state);
        self.wake();
        true
    }

    /// Asks the window to draw again.
    fn wake(&self) {
        if let Some(window) = lock(&self.window).clone() {
            window.request_redraw();
        }
    }

    /// The window the session's widget is in, woken when a frame arrives.
    pub fn attach_window(&self, window: Option<Arc<dyn ShellProvider>>) {
        *lock(&self.window) = window;
    }

    fn send(&self, message: &ToGuest) {
        let socket = lock(&self.state).socket.clone();
        if let (Some(socket), Ok(bytes)) = (socket, message.encode()) {
            // A program that stopped reading: the reader thread notices
            // when it exits; nothing here waits for it.
            let _ = socket.send(&bytes, &[]);
        }
    }

    /// One input event for the program.
    pub fn input(&self, input: ViewInput) {
        let seq = {
            let mut state = lock(&self.state);
            state.input_seq = state.input_seq.saturating_add(1);
            let seq = state.input_seq;
            let now = Instant::now();
            state.sent.push_back((seq, now));
            while state.sent.len() > 256 {
                state.sent.pop_front();
            }
            state.unanswered.get_or_insert(now);
            seq
        };
        self.send(&ToGuest::Input { seq, input });
    }

    /// The app's theme, sent when it changed.
    pub fn theme(&self, theme: ThemeTokens) {
        let changed = {
            let mut state = lock(&self.state);
            let changed = state.theme != theme;
            state.theme = theme.clone();
            changed && state.hello.is_some()
        };
        if changed {
            self.send(&ToGuest::Theme(theme));
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        let size = state.buffers.as_ref().and_then(|b| match b.message {
            ToHost::Buffers { width, height, .. } => Some((width, height)),
            ToHost::Frame { .. } => None,
        });
        Snapshot {
            status: state.status.clone(),
            pid: state.pid,
            generation: state.generation,
            transport: state.transport,
            size,
            frames: state.frames_shown,
            render: self.stats.render.summary(),
            input_to_frame: self.stats.input_to_frame.summary(),
            input_to_paint: self.stats.input_to_paint.summary(),
            host_paint: self.stats.host_paint.summary(),
            waiting: state.unanswered.map(|at| at.elapsed()),
            adapter: state.adapter.clone(),
        }
    }
}

/// When input `seq` was sent, forgetting it and every one before it.
fn sent_at(sent: &mut VecDeque<(u64, Instant)>, seq: u64) -> Option<Instant> {
    let mut at = None;
    while let Some(&(s, t)) = sent.front() {
        if s > seq {
            break;
        }
        if s == seq {
            at = Some(t);
        }
        sent.pop_front();
    }
    at
}

/// How `child` ended, if it did within a moment.
fn wait_briefly(child: &mut Child) -> Option<String> {
    use std::os::unix::process::ExitStatusExt as _;
    for _ in 0..50 {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(match (status.code(), status.signal()) {
                (Some(code), _) => format!("it exited with status {code}"),
                (None, Some(signal)) => format!("it was killed (signal {signal})"),
                (None, None) => "it exited".into(),
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

// ── The widget ───────────────────────────────────────────────────────

/// The frames of one set of buffers, in the window's device.
enum Imported {
    /// One registered texture per slot.
    Dmabuf { ids: Vec<ResourceId> },
    /// One texture frames are uploaded into, and the program's memory.
    Shm {
        id: ResourceId,
        texture: wgpu::Texture,
        map: Map,
        stride: u32,
    },
}

struct Map {
    ptr: *mut std::ffi::c_void,
    len: usize,
}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: mapped by `import` with this length.
        let _ = unsafe { rustix::mm::munmap(self.ptr, self.len) };
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wire::Adapter,
}

/// A process view as a Blitz custom widget.
pub struct ProcessViewWidget {
    session: Arc<Session>,
    gpu: Option<Gpu>,
    generation: u64,
    /// The buffers imported, by id, their size and what they became.
    imported: Option<(u64, (u32, u32), Imported)>,
    /// The slot drawn, and the one before it (the GPU may still read it).
    shown: VecDeque<(u64, u8)>,
    current: Option<(u64, u8)>,
    fresh: bool,
}

impl std::fmt::Debug for ProcessViewWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProcessViewWidget")
            .field(&self.session)
            .finish()
    }
}

impl ProcessViewWidget {
    #[must_use]
    pub const fn new(session: Arc<Session>) -> Self {
        Self {
            session,
            gpu: None,
            generation: 0,
            imported: None,
            shown: VecDeque::new(),
            current: None,
            fresh: true,
        }
    }

    fn release(&self, (buffers, slot): (u64, u8)) {
        self.session.send(&ToGuest::Release { buffers, slot });
    }

    fn forget(&mut self, ctx: &mut dyn RenderContext) {
        match self.imported.take() {
            Some((_, _, Imported::Dmabuf { ids })) => {
                for id in ids {
                    ctx.unregister_resource(id);
                }
            }
            Some((_, _, Imported::Shm { id, .. })) => ctx.unregister_resource(id),
            None => {}
        }
        self.shown.clear();
        self.current = None;
    }

    /// Hello (once per start), or an expose (a new widget for a running
    /// program), and the size when it changed.
    fn greet(&mut self, gpu: &Gpu, (width, height, scale): (u32, u32, f32)) {
        let (hello, resize, theme) = {
            let mut state = lock(&self.session.state);
            let hello = state.hello != Some(state.generation) && state.socket.is_some();
            if hello {
                state.hello = Some(state.generation);
                state.size = Some((width, height, scale));
                state.adapter = Some(gpu.adapter.name.clone());
            }
            let resize = !hello && state.size != Some((width, height, scale));
            if resize {
                state.size = Some((width, height, scale));
            }
            (hello, resize, state.theme.clone())
        };
        if hello {
            self.session.send(&ToGuest::Hello {
                version: wire::VERSION,
                width,
                height,
                scale,
                adapter: gpu.adapter.clone(),
                dmabuf: gpu::can_share(&gpu.device),
                theme,
            });
        } else if resize {
            self.session.send(&ToGuest::Resize {
                width,
                height,
                scale,
            });
        } else if self.fresh {
            self.session.send(&ToGuest::Expose);
        }
        self.fresh = false;
    }

    /// The announced buffers in the window's device, if they are new.
    fn import(&mut self, gpu: &Gpu, ctx: &mut dyn RenderContext) {
        let state = lock(&self.session.state);
        let Some(Announced { message, fds }) = &state.buffers else {
            return;
        };
        let ToHost::Buffers {
            id,
            transport,
            width,
            height,
            slots,
            stride,
            offset,
            modifier,
        } = *message
        else {
            return;
        };
        if self.imported.as_ref().is_some_and(|(have, ..)| *have == id) {
            return;
        }
        let dup: Vec<OwnedFd> = fds.iter().filter_map(|fd| fd.try_clone().ok()).collect();
        drop(state);
        self.forget(ctx);
        let imported = match transport {
            Transport::Dmabuf => {
                let mut ids = Vec::new();
                for fd in dup.into_iter().take(usize::from(slots)) {
                    match gpu::import_dmabuf(
                        &gpu.device,
                        fd,
                        (width, height),
                        (stride, offset, modifier),
                    ) {
                        Ok(texture) => match ctx.try_register_custom_resource(Box::new(texture)) {
                            Ok(rid) => ids.push(rid),
                            Err(_) => break,
                        },
                        Err(_) => break,
                    }
                }
                if ids.len() != usize::from(slots) {
                    for rid in ids {
                        ctx.unregister_resource(rid);
                    }
                    // This window can't import them: ask for shared memory.
                    self.fallback(gpu, (width, height));
                    return;
                }
                Imported::Dmabuf { ids }
            }
            Transport::Shm => {
                let Some(fd) = dup.into_iter().next() else {
                    return;
                };
                let Some(len) = u64::from(stride)
                    .checked_mul(u64::from(height))
                    .and_then(|l| l.checked_mul(u64::from(slots)))
                    .and_then(|l| usize::try_from(l).ok())
                else {
                    return;
                };
                // SAFETY: a read-only shared mapping of the program's
                // frames, `len` long as it announced; the program writes
                // a slot only while the app doesn't hold it.
                let Ok(ptr) = (unsafe {
                    rustix::mm::mmap(
                        std::ptr::null_mut(),
                        len,
                        rustix::mm::ProtFlags::READ,
                        rustix::mm::MapFlags::SHARED,
                        fd.as_fd(),
                        0,
                    )
                }) else {
                    return;
                };
                let texture = gpu::upload_texture(&gpu.device, width, height);
                let Ok(rid) = ctx.try_register_custom_resource(Box::new(texture.clone())) else {
                    return;
                };
                Imported::Shm {
                    id: rid,
                    texture,
                    map: Map { ptr, len },
                    stride,
                }
            }
        };
        self.imported = Some((id, (width, height), imported));
    }

    /// Says hello again without dma-bufs: the program switches to
    /// shared memory.
    fn fallback(&self, gpu: &Gpu, (width, height): (u32, u32)) {
        let (scale, theme) = {
            let state = lock(&self.session.state);
            (state.size.map_or(1.0, |s| s.2), state.theme.clone())
        };
        self.session.send(&ToGuest::Hello {
            version: wire::VERSION,
            width,
            height,
            scale,
            adapter: gpu.adapter.clone(),
            dmabuf: false,
            theme,
        });
    }

    /// The newest frame of the imported buffers: shown (uploaded first
    /// from shared memory), the others released.
    fn take_frame(&mut self, gpu: &Gpu) {
        let Some((imported_id, (width, height), imported)) = &self.imported else {
            return;
        };
        let arrived: Vec<Arrived> = {
            let mut state = lock(&self.session.state);
            state.frames.drain(..).collect()
        };
        let Some(newest) = arrived
            .iter()
            .rev()
            .find(|f| f.buffers == *imported_id)
            .copied()
        else {
            return;
        };
        for f in &arrived {
            if f.buffers == *imported_id && f.slot != newest.slot {
                self.release((f.buffers, f.slot));
            }
        }
        match imported {
            Imported::Dmabuf { .. } => {
                if let Some(previous) = self.current.replace((newest.buffers, newest.slot)) {
                    self.shown.push_back(previous);
                }
                while self.shown.len() > 1 {
                    if let Some(old) = self.shown.pop_front() {
                        self.release(old);
                    }
                }
            }
            Imported::Shm {
                texture,
                map,
                stride,
                ..
            } => {
                let slot_len = u64::from(*stride).saturating_mul(u64::from(*height));
                let start = usize::try_from(slot_len.saturating_mul(u64::from(newest.slot)))
                    .unwrap_or(usize::MAX);
                let len = usize::try_from(slot_len).unwrap_or(usize::MAX);
                if start.checked_add(len).is_some_and(|end| end <= map.len) {
                    // SAFETY: inside the mapping (checked); the program
                    // doesn't write this slot until it is released below.
                    let bytes =
                        unsafe { std::slice::from_raw_parts(map.ptr.cast::<u8>().add(start), len) };
                    gpu.queue.write_texture(
                        texture.as_image_copy(),
                        bytes,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(*stride),
                            rows_per_image: Some(*height),
                        },
                        wgpu::Extent3d {
                            width: *width,
                            height: *height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                // `write_texture` copied it: the slot is free again.
                self.release((newest.buffers, newest.slot));
                self.current = Some((newest.buffers, newest.slot));
            }
        }
        let mut state = lock(&self.session.state);
        state.frames_shown = state.frames_shown.saturating_add(1);
        drop(state);
        // Shown: the program may draw the next one.
        self.session.send(&ToGuest::Tick);
        // The frames skipped since count too: the paint shows them.
        if let Some(at) = arrived.iter().filter_map(|f| f.sent).min() {
            self.session.stats.input_to_paint.record(at.elapsed());
        }
    }
}

impl Widget for ProcessViewWidget {
    fn handle_event(&mut self, event: &UiEvent) {
        if let Some(input) = view_input(event) {
            self.session.input(input);
        }
    }

    fn requires_redraw(&self) -> bool {
        !lock(&self.session.state).frames.is_empty()
    }

    fn destroy_surfaces(&mut self) {
        self.gpu = None;
        self.imported = None;
        self.shown.clear();
        self.current = None;
    }

    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    fn paint(
        &mut self,
        ctx: &mut dyn RenderContext,
        _: &ComputedStyles,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Scene {
        let began = Instant::now();
        if self.gpu.is_none()
            && let Some(handle) = ctx
                .renderer_specific_context()
                .and_then(|c| c.downcast::<wgpu_context::DeviceHandle>().ok())
        {
            self.gpu = Some(Gpu {
                adapter: gpu::adapter_info(&handle.adapter),
                device: handle.device.clone(),
                queue: handle.queue.clone(),
            });
        }
        let generation = lock(&self.session.state).generation;
        if generation != self.generation {
            self.forget(ctx);
            self.generation = generation;
            self.fresh = true;
        }
        let mut scene = Scene::new();
        let all = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(12, 12, 14, 255),
            None,
            &all,
        );
        let Some(gpu) = self.gpu.take() else {
            return scene;
        };
        if width > 0 && height > 0 {
            self.greet(&gpu, (width, height, scale as f32));
        }
        self.import(&gpu, ctx);
        self.take_frame(&gpu);
        self.gpu = Some(gpu);
        if let (Some((buffers, slot)), Some((id, (w, h), imported))) =
            (self.current, &self.imported)
            && buffers == *id
        {
            let rid = match imported {
                Imported::Dmabuf { ids } => ids.get(usize::from(slot)).copied(),
                Imported::Shm { id, .. } => Some(*id),
            };
            if let Some(rid) = rid {
                // At its own size: while a resize is under way the old
                // frame stays sharp, clipped to the pane.
                let shape = Rect::new(
                    0.0,
                    0.0,
                    f64::from(*w).min(all.x1),
                    f64::from(*h).min(all.y1),
                );
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    anyrender::PaintRef::Resource(ImageBrush {
                        image: rid,
                        sampler: ImageSampler::default(),
                    }),
                    None,
                    &shape,
                );
            }
        }
        self.session.stats.host_paint.record(began.elapsed());
        scene
    }
}

impl Drop for ProcessViewWidget {
    fn drop(&mut self) {
        // The pane went (or moved): the program may draw into these again.
        for held in self.shown.drain(..).chain(self.current.take()) {
            self.session.send(&ToGuest::Release {
                buffers: held.0,
                slot: held.1,
            });
        }
    }
}

// ── The pane ─────────────────────────────────────────────────────────

/// How often the pane's line looks at its session.
const POLL: Duration = Duration::from_millis(200);

/// The pane: the program's view, a line of what it measures, and when
/// it stopped, why, with a button that starts it again.
#[component]
pub fn ProcessViewPane(
    /// The session's key (the pane's id: the program survives the pane
    /// moving).
    session_key: String,
    program: Program,
) -> Element {
    let session = use_hook(|| session(&session_key, &program));
    let window = try_use_context::<Arc<dyn ShellProvider>>();
    session.attach_window(window.clone());
    let widget = use_hook({
        let session = session.clone();
        move || CustomWidgetAttr::new(ProcessViewWidget::new(session))
    });
    let mut snap = use_signal({
        let session = session.clone();
        move || session.snapshot()
    });
    use_future({
        let session = session.clone();
        move || {
            let session = session.clone();
            async move {
                loop {
                    futures_timer::Delay::new(POLL).await;
                    snap.set(session.snapshot());
                }
            }
        }
    });
    let s = snap();
    let stopped = match &s.status {
        Status::Stopped(why) => Some(why.clone()),
        _ => None,
    };
    let hung = s.waiting.is_some_and(|w| w > HANG) && stopped.is_none();
    let focus = {
        let session = session.clone();
        let window = window.clone();
        move |on: bool| {
            session.input(ViewInput::Focus(on));
            if let Some(window) = &window {
                window.request_redraw();
            }
        }
    };
    let blur = focus.clone();
    let restart = session.clone();
    let line = status_line(&s);
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
            div { style: "flex: 1; position: relative; min-height: 0; overflow: hidden;",
                object {
                    style: "position: absolute; inset: 0; display: block; width: 100%; height: 100%; outline: none;",
                    tabindex: "0",
                    data: widget,
                    onfocus: move |_| focus(true),
                    onblur: move |_| blur(false),
                }
                if let Some(why) = stopped {
                    div { style: "position: absolute; inset: 0; display: flex; flex-direction: column; \
                                  align-items: center; justify-content: center; gap: 10px; \
                                  background: rgba(12,12,14,0.86); color: #e4e4e7; font-size: 14px;",
                        div { style: "font-weight: 600;", "View stopped" }
                        div { style: "color: #a1a1aa; font-size: 12px; max-width: 80%; text-align: center;", "{why}" }
                        button {
                            style: "padding: 6px 14px; border-radius: 6px; border: 1px solid #3f3f46; \
                                    background: #27272a; color: #fafafa; font-size: 13px;",
                            onclick: move |_| restart.restart(),
                            "Restart"
                        }
                    }
                }
                if hung {
                    div { style: "position: absolute; top: 8px; right: 8px; padding: 3px 8px; border-radius: 4px; \
                                  background: #7c2d12; color: #fff7ed; font-size: 12px;",
                        "Not responding"
                    }
                }
            }
            div { style: "padding: 4px 8px; font-size: 11px; color: #a1a1aa; font-family: monospace;",
                "{line}"
            }
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// The pane's line: the program, its frames and its latency.
#[must_use]
pub fn status_line(s: &Snapshot) -> String {
    let transport = match s.transport {
        Some(Transport::Dmabuf) => "dma-buf",
        Some(Transport::Shm) => "shared memory",
        None => "–",
    };
    let size = s
        .size
        .map_or_else(|| "–".into(), |(w, h)| format!("{w}×{h}"));
    let state = match &s.status {
        Status::Starting => "starting",
        Status::Running => "running",
        Status::Stopped(_) => "stopped",
    };
    format!(
        "{state} · pid {} · {transport} {size} · {} frames · guest p50 {:.2} ms · input→frame p50 {:.2} / p99 {:.2} ms · input→paint p50 {:.2} ms · host paint p50 {:.3} ms",
        s.pid.map_or_else(|| "–".into(), |p| p.to_string()),
        s.frames,
        ms(s.render.p50),
        ms(s.input_to_frame.p50),
        ms(s.input_to_frame.p99),
        ms(s.input_to_paint.p50),
        ms(s.host_paint.p50),
    )
}
