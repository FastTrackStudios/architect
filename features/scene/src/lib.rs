//! Scenes as they cross a boundary.
//!
//! A view that runs outside the app's own code (a process, a WASI
//! component, a page) records what it draws as anyrender commands, as a
//! linked view does, and sends them to the host that draws them. This
//! crate is that crossing:
//!
//! - A [`fragment`] is a list of drawing commands as bytes: fills,
//!   strokes, clips, layers, box shadows and glyph runs, with gradients
//!   and images as they were drawn. It is named by the hash of its bytes
//!   ([`Id`]), so each side can tell what the other already holds.
//! - A [`resource`] is a font's file or an image's pixels, named by the
//!   hash of its bytes and sent once. A fragment names the resources it
//!   draws; it never carries them.
//! - [`layers`] are what a pane's host keeps of it: content by key, and
//!   where each piece is drawn, composed every frame without the view.
//! - [`anim`] is how a layer moves without the view: a property as a
//!   function of a clock (wall time, the app's playhead) or of a named
//!   value (a meter, a fader), evaluated by the host each frame.
//!
//! The host hashes what it receives itself: a name is never the sender's
//! word, so one extension can't put its bytes under another's name.
//!
//! What crosses is what was recorded: a path's coordinates are kept as
//! `f32` only when that loses nothing, so a view draws the same pixels
//! linked in or at run time. What can't cross is counted
//! ([`fragment::Dropped`]), not silently lost: a layer's filters and a
//! backend's own paints.
//!
//! Everything a host reads is checked ([`Error`]): sizes, counts, nesting
//! and every number, so what a view sends can fail the view, never the
//! app.

pub mod anim;
mod bytes;
pub mod fragment;
pub mod layers;
pub mod ops;
pub mod resource;

pub use anim::{Animation, Clock, Clocks, Input, Linear, Map, Property, Readings, Values};
pub use anyrender::recording::RenderCommand;
pub use fragment::{Dropped, Encoded, Encoder, decode};
pub use layers::{Layer, Layers};
pub use resource::{Resource, Resources};

/// The most bytes one fragment may be.
pub const MAX_FRAGMENT_BYTES: usize = 16 * 1024 * 1024;

/// The most commands one fragment may hold.
pub const MAX_COMMANDS: usize = 100_000;

/// The most path elements (moves, lines, curves) one fragment may hold.
pub const MAX_PATH_ELEMENTS: usize = 1_000_000;

/// The most glyphs one fragment may hold.
pub const MAX_GLYPHS: usize = 1_000_000;

/// The most layers and clips a fragment may have open at once.
pub const MAX_DEPTH: usize = 64;

/// The largest coordinate, transform term or size a fragment may use:
/// past it (or not finite) a renderer can stall or overflow.
pub const MAX_COORDINATE: f64 = 1.0e7;

/// The most bytes one font's file may be.
pub const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;

/// The longest side of an image, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 8192;

/// What a fragment or a resource is named by: the first 16 bytes of the
/// BLAKE3 hash of its bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub [u8; 16]);

impl Id {
    /// The name of `parts`, hashed in order.
    #[must_use]
    pub fn of(parts: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        for part in parts {
            hasher.update(part);
        }
        let mut id = [0_u8; 16];
        for (to, from) in id.iter_mut().zip(hasher.finalize().as_bytes()) {
            *to = *from;
        }
        Self(id)
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Id({self})")
    }
}

/// What is wrong with a fragment or a resource a host was sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("it ends before what it holds does")]
    Truncated,
    #[error("it is not a fragment (or one of a version this host doesn't read)")]
    NotAFragment,
    #[error("{0} bytes: past its limit of {1}")]
    TooLarge(usize, usize),
    #[error("more than {0} commands")]
    TooManyCommands(usize),
    #[error("more than {0} path elements")]
    TooManyPathElements(usize),
    #[error("more than {0} glyphs")]
    TooManyGlyphs(usize),
    #[error("layers nested past {0}")]
    TooDeep(usize),
    #[error("a layer popped that it never pushed")]
    UnbalancedPop,
    #[error("a number out of range, or not finite")]
    OutOfRange,
    #[error("a value it doesn't know: {0} = {1}")]
    Unknown(&'static str, u8),
    #[error("a resource {0} it was never sent, or no longer holds")]
    MissingResource(Id),
    #[error("an image of {0} × {1} pixels whose bytes are not that many")]
    ImageSize(u32, u32),
    #[error("more than {0} layers")]
    TooManyLayers(usize),
    #[error("{0} bytes after its last command")]
    Trailing(usize),
    #[error("an animation that can't be run: {0}")]
    BadAnimation(&'static str),
    #[error("more than {0} animations")]
    TooManyAnimations(usize),
    #[error("not a value's name: {0:?}")]
    BadName(String),
    #[error("more than {0} values")]
    TooManyValues(usize),
    #[error("a value it may not {0}: {1}")]
    Denied(&'static str, String),
}
