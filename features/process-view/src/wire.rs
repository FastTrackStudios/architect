//! The messages between the app (the host) and a view's program (the
//! guest), one per datagram on a `SOCK_SEQPACKET` pair
//! ([`crate::sock`]): little-endian, a tag byte first. File descriptors
//! (the frames' dma-bufs, or the shared memory) travel beside a message
//! as `SCM_RIGHTS`, never in it.
//!
//! The host asks: [`ToGuest::Hello`] once (the pane's size, its adapter,
//! what it can import), then [`ToGuest::Resize`], [`ToGuest::Input`],
//! [`ToGuest::Release`] and [`ToGuest::Expose`] as the pane lives. The
//! guest answers with [`ToHost::Buffers`] (with their descriptors) for
//! every size, and a [`ToHost::Frame`] for every picture.

use architect_workspace::{Button, ThemeTokens, ViewInput};

/// The protocol's version; a guest refuses a host with another.
pub const VERSION: u16 = 1;

/// At most this many bytes in one message.
pub const MAX_MESSAGE: usize = 64 * 1024;

/// At most this many frames' slots in one set of buffers.
pub const MAX_SLOTS: u8 = 8;

/// How the frames are shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// One dma-buf per slot, a texture both sides' GPU devices import
    /// (Vulkan external memory, a DRM format modifier): no copy.
    Dmabuf,
    /// One shared-memory file for every slot: the guest reads its frame
    /// back from its GPU and the host uploads it to its own.
    Shm,
}

/// The GPU adapter the host draws with, so the guest renders on the
/// same one (a dma-buf only imports on the device that can read it).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Adapter {
    pub vendor: u32,
    pub device: u32,
    /// The adapter's name, as wgpu reports it.
    pub name: String,
}

/// From the host.
#[derive(Debug, Clone, PartialEq)]
pub enum ToGuest {
    /// The pane, the first time: its size in physical pixels and the
    /// display's scale, the host's adapter, whether it can import a
    /// dma-buf, and the app's theme tokens.
    Hello {
        version: u16,
        width: u32,
        height: u32,
        scale: f32,
        adapter: Adapter,
        dmabuf: bool,
        theme: ThemeTokens,
    },
    /// A new size (physical pixels) or scale.
    Resize { width: u32, height: u32, scale: f32 },
    /// One input event, in the view's logical pixels, numbered so the
    /// frame that shows it says so.
    Input { seq: u64, input: ViewInput },
    /// The host is done with `slot` of `buffers`: the guest may draw
    /// into it again.
    Release { buffers: u64, slot: u8 },
    /// The host lost its picture (a new widget for the same view: the
    /// pane moved): draw one now, whatever changed.
    Expose,
    /// The app's theme changed.
    Theme(ThemeTokens),
    /// The app drew the last frame: the program may draw the next one (a
    /// frame callback, as Wayland's `wl_surface.frame`). One frame is
    /// in flight at a time, so a frame shows the newest input instead of
    /// queueing behind older frames.
    Tick,
}

/// From the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToHost {
    /// A new set of buffers, `id` numbering them; sent with one dma-buf
    /// per slot, or with one shared-memory file holding every slot,
    /// `stride * height` bytes apart.
    Buffers {
        id: u64,
        transport: Transport,
        width: u32,
        height: u32,
        slots: u8,
        /// Bytes from one row to the next.
        stride: u32,
        /// Where the first row is (a dma-buf's plane offset).
        offset: u32,
        /// The DRM format modifier (a dma-buf's tiling; 0 is linear).
        modifier: u64,
    },
    /// A picture in `slot` of buffers `buffers`, which the host holds
    /// until it sends [`ToGuest::Release`]. `input` is the last input
    /// event it shows (0 for none); `render_us` what it took the guest,
    /// from the frame's start to its pixels being ready.
    Frame {
        buffers: u64,
        slot: u8,
        seq: u64,
        input: u64,
        render_us: u32,
    },
}

/// A message that could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    #[error("the message ends early")]
    Short,
    #[error("unknown message tag {0}")]
    Tag(u8),
    #[error("a string that is not UTF-8")]
    Utf8,
    #[error("the message is larger than {MAX_MESSAGE} bytes")]
    TooLarge,
    #[error("a value out of range")]
    Range,
}

// ── Encoding ─────────────────────────────────────────────────────────

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.u32(v.to_bits())
    }
    fn str(&mut self, v: &str) -> &mut Self {
        let len = u32::try_from(v.len()).unwrap_or(u32::MAX);
        self.u32(len);
        self.0.extend_from_slice(v.as_bytes());
        self
    }
    fn strs(&mut self, v: &[&str]) -> &mut Self {
        self.u8(u8::try_from(v.len()).unwrap_or(u8::MAX));
        for s in v.iter().take(usize::from(u8::MAX)) {
            self.str(s);
        }
        self
    }
    fn theme(&mut self, t: &ThemeTokens) -> &mut Self {
        self.u32(u32::try_from(t.0.len()).unwrap_or(u32::MAX));
        for (k, v) in &t.0 {
            self.str(k).str(v);
        }
        self
    }
    fn done(self) -> Result<Vec<u8>, WireError> {
        if self.0.len() > MAX_MESSAGE {
            return Err(WireError::TooLarge);
        }
        Ok(self.0)
    }
}

struct In<'a>(&'a [u8]);

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(WireError::Short)?;
        self.0 = rest;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        self.take(N)?.try_into().map_err(|_| WireError::Short)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(u8::from_le_bytes(self.array()?))
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, WireError> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn string(&mut self) -> Result<String, WireError> {
        let len = usize::try_from(self.u32()?).map_err(|_| WireError::Range)?;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| WireError::Utf8)
    }
    fn names(&mut self) -> Result<Vec<&'static str>, WireError> {
        let n = self.u8()?;
        (0..n).map(|_| Ok(static_name(&self.string()?))).collect()
    }
    fn theme(&mut self) -> Result<ThemeTokens, WireError> {
        let n = self.u32()?;
        let mut t = ThemeTokens::default();
        for _ in 0..n {
            let (k, v) = (self.string()?, self.string()?);
            t.0.insert(k, v);
        }
        Ok(t)
    }
}

/// The modifier and phase names [`ViewInput`] holds as `&'static str`.
const NAMES: [&str; 8] = [
    "Mod", "Shift", "Alt", "Ctrl", "start", "move", "end", "cancel",
];

fn static_name(s: &str) -> &'static str {
    NAMES.iter().find(|n| **n == s).copied().unwrap_or("")
}

const fn button_code(b: Button) -> u8 {
    match b {
        Button::Primary => 0,
        Button::Secondary => 1,
        Button::Middle => 2,
    }
}

const fn button_from(code: u8) -> Button {
    match code {
        1 => Button::Secondary,
        2 => Button::Middle,
        _ => Button::Primary,
    }
}

fn input(out: &mut Out, event: &ViewInput) {
    match event {
        ViewInput::PointerDown { x, y, button } => {
            out.u8(0).f32(*x).f32(*y).u8(button_code(*button));
        }
        ViewInput::PointerMove { x, y } => {
            out.u8(1).f32(*x).f32(*y);
        }
        ViewInput::PointerUp { x, y, button } => {
            out.u8(2).f32(*x).f32(*y).u8(button_code(*button));
        }
        ViewInput::Wheel { dx, dy, x, y, mods } => {
            out.u8(3).f32(*dx).f32(*dy).f32(*x).f32(*y).strs(mods);
        }
        ViewInput::Touch { id, x, y, phase } => {
            out.u8(4).u64(*id).f32(*x).f32(*y).str(phase);
        }
        ViewInput::Key {
            key,
            down,
            modifiers,
        } => {
            out.u8(5).str(key).u8(u8::from(*down)).strs(modifiers);
        }
        ViewInput::Ime { preedit, commit } => {
            out.u8(6).str(preedit);
            match commit {
                Some(c) => out.u8(1).str(c),
                None => out.u8(0),
            };
        }
        ViewInput::Focus(on) => {
            out.u8(7).u8(u8::from(*on));
        }
    }
}

fn read_input(i: &mut In<'_>) -> Result<ViewInput, WireError> {
    Ok(match i.u8()? {
        0 => ViewInput::PointerDown {
            x: i.f32()?,
            y: i.f32()?,
            button: button_from(i.u8()?),
        },
        1 => ViewInput::PointerMove {
            x: i.f32()?,
            y: i.f32()?,
        },
        2 => ViewInput::PointerUp {
            x: i.f32()?,
            y: i.f32()?,
            button: button_from(i.u8()?),
        },
        3 => ViewInput::Wheel {
            dx: i.f32()?,
            dy: i.f32()?,
            x: i.f32()?,
            y: i.f32()?,
            mods: i.names()?,
        },
        4 => ViewInput::Touch {
            id: i.u64()?,
            x: i.f32()?,
            y: i.f32()?,
            phase: static_name(&i.string()?),
        },
        5 => ViewInput::Key {
            key: i.string()?,
            down: i.u8()? != 0,
            modifiers: i.names()?,
        },
        6 => {
            let preedit = i.string()?;
            let commit = if i.u8()? == 0 {
                None
            } else {
                Some(i.string()?)
            };
            ViewInput::Ime { preedit, commit }
        }
        7 => ViewInput::Focus(i.u8()? != 0),
        tag => return Err(WireError::Tag(tag)),
    })
}

impl ToGuest {
    /// The message's bytes.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut out = Out::default();
        match self {
            Self::Hello {
                version,
                width,
                height,
                scale,
                adapter,
                dmabuf,
                theme,
            } => {
                out.u8(1)
                    .u16(*version)
                    .u32(*width)
                    .u32(*height)
                    .f32(*scale)
                    .u32(adapter.vendor)
                    .u32(adapter.device)
                    .str(&adapter.name)
                    .u8(u8::from(*dmabuf))
                    .theme(theme);
            }
            Self::Resize {
                width,
                height,
                scale,
            } => {
                out.u8(2).u32(*width).u32(*height).f32(*scale);
            }
            Self::Input { seq, input: event } => {
                out.u8(3).u64(*seq);
                input(&mut out, event);
            }
            Self::Release { buffers, slot } => {
                out.u8(4).u64(*buffers).u8(*slot);
            }
            Self::Expose => {
                out.u8(5);
            }
            Self::Theme(theme) => {
                out.u8(6).theme(theme);
            }
            Self::Tick => {
                out.u8(7);
            }
        }
        out.done()
    }

    /// The message in `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut i = In(bytes);
        Ok(match i.u8()? {
            1 => Self::Hello {
                version: i.u16()?,
                width: i.u32()?,
                height: i.u32()?,
                scale: i.f32()?,
                adapter: Adapter {
                    vendor: i.u32()?,
                    device: i.u32()?,
                    name: i.string()?,
                },
                dmabuf: i.u8()? != 0,
                theme: i.theme()?,
            },
            2 => Self::Resize {
                width: i.u32()?,
                height: i.u32()?,
                scale: i.f32()?,
            },
            3 => Self::Input {
                seq: i.u64()?,
                input: read_input(&mut i)?,
            },
            4 => Self::Release {
                buffers: i.u64()?,
                slot: i.u8()?,
            },
            5 => Self::Expose,
            6 => Self::Theme(i.theme()?),
            7 => Self::Tick,
            tag => return Err(WireError::Tag(tag)),
        })
    }
}

impl ToHost {
    /// The message's bytes.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut out = Out::default();
        match *self {
            Self::Buffers {
                id,
                transport,
                width,
                height,
                slots,
                stride,
                offset,
                modifier,
            } => {
                out.u8(1)
                    .u64(id)
                    .u8(match transport {
                        Transport::Dmabuf => 0,
                        Transport::Shm => 1,
                    })
                    .u32(width)
                    .u32(height)
                    .u8(slots)
                    .u32(stride)
                    .u32(offset)
                    .u64(modifier);
            }
            Self::Frame {
                buffers,
                slot,
                seq,
                input,
                render_us,
            } => {
                out.u8(2)
                    .u64(buffers)
                    .u8(slot)
                    .u64(seq)
                    .u64(input)
                    .u32(render_us);
            }
        }
        out.done()
    }

    /// The message in `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut i = In(bytes);
        Ok(match i.u8()? {
            1 => {
                let id = i.u64()?;
                let transport = match i.u8()? {
                    0 => Transport::Dmabuf,
                    1 => Transport::Shm,
                    _ => return Err(WireError::Range),
                };
                let (width, height, slots) = (i.u32()?, i.u32()?, i.u8()?);
                if slots == 0 || slots > MAX_SLOTS {
                    return Err(WireError::Range);
                }
                Self::Buffers {
                    id,
                    transport,
                    width,
                    height,
                    slots,
                    stride: i.u32()?,
                    offset: i.u32()?,
                    modifier: i.u64()?,
                }
            }
            2 => Self::Frame {
                buffers: i.u64()?,
                slot: i.u8()?,
                seq: i.u64()?,
                input: i.u64()?,
                render_us: i.u32()?,
            },
            tag => return Err(WireError::Tag(tag)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_guest(m: &ToGuest) {
        assert_eq!(&ToGuest::decode(&m.encode().unwrap()).unwrap(), m);
    }

    #[test]
    fn every_host_message_round_trips() {
        round_trip_guest(&ToGuest::Hello {
            version: VERSION,
            width: 1920,
            height: 1080,
            scale: 1.5,
            adapter: Adapter {
                vendor: 0x10de,
                device: 0x2704,
                name: "NVIDIA GeForce RTX 4080".into(),
            },
            dmabuf: true,
            theme: ThemeTokens::default().with("brand", "#ff7a1a"),
        });
        round_trip_guest(&ToGuest::Resize {
            width: 3,
            height: 4,
            scale: 2.0,
        });
        for input in [
            ViewInput::PointerDown {
                x: 1.0,
                y: 2.0,
                button: Button::Secondary,
            },
            ViewInput::PointerMove { x: 1.5, y: -2.0 },
            ViewInput::PointerUp {
                x: 1.0,
                y: 2.0,
                button: Button::Middle,
            },
            ViewInput::Wheel {
                dx: 0.0,
                dy: 20.0,
                x: 4.0,
                y: 5.0,
                mods: vec!["Shift", "Mod"],
            },
            ViewInput::Touch {
                id: 7,
                x: 1.0,
                y: 1.0,
                phase: "move",
            },
            ViewInput::Key {
                key: "é".into(),
                down: true,
                modifiers: vec!["Alt"],
            },
            ViewInput::Ime {
                preedit: "か".into(),
                commit: Some("火".into()),
            },
            ViewInput::Focus(true),
        ] {
            round_trip_guest(&ToGuest::Input { seq: 9, input });
        }
        round_trip_guest(&ToGuest::Release {
            buffers: 2,
            slot: 3,
        });
        round_trip_guest(&ToGuest::Expose);
        round_trip_guest(&ToGuest::Tick);
    }

    #[test]
    fn every_guest_message_round_trips() {
        for m in [
            ToHost::Buffers {
                id: 1,
                transport: Transport::Dmabuf,
                width: 640,
                height: 480,
                slots: 4,
                stride: 2560,
                offset: 0,
                modifier: 0,
            },
            ToHost::Frame {
                buffers: 1,
                slot: 2,
                seq: 99,
                input: 12,
                render_us: 900,
            },
        ] {
            assert_eq!(ToHost::decode(&m.encode().unwrap()).unwrap(), m);
        }
    }

    #[test]
    fn a_truncated_or_unknown_message_is_an_error_not_a_panic() {
        let bytes = ToHost::Frame {
            buffers: 1,
            slot: 0,
            seq: 1,
            input: 0,
            render_us: 1,
        }
        .encode()
        .unwrap();
        for n in 0..bytes.len() {
            assert!(ToHost::decode(&bytes[..n]).is_err());
        }
        assert_eq!(ToHost::decode(&[9]), Err(WireError::Tag(9)));
        assert!(ToGuest::decode(&[3, 0, 0, 0, 0, 0, 0, 0, 0, 42]).is_err());
        let zero_slots = [1u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(ToHost::decode(&zero_slots).is_err());
    }
}
