//! Renditions: media derived from a file's version.
//!
//! A video's streaming proxy and poster, an audio file's peaks and its
//! browser-playable transcode, made off the request path by the server's
//! transcoder and kept with the bytes they came from.
//!
//! A rendition is addressed by the version it was made from (a path, and
//! a version or the current one) and keyed underneath by that version's
//! content, so two files holding the same bytes share their renditions,
//! and a recipe change ([`RECIPE`]) makes every rendition again rather
//! than serving one made the old way.

use std::fmt;
use std::str::FromStr;

use facet::Facet;

use crate::id::ContentId;

/// The recipe every rendition is made by. Bumped when what a kind *is*
/// changes (a codec, a size, the peaks' resolution): renditions made by
/// another recipe are then missing, and made again.
pub const RECIPE: u32 = 1;

/// One kind of derived media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[repr(u8)]
pub enum RenditionKind {
    /// A video's streaming proxy: H.264 and AAC in an MP4 with its index
    /// up front, at most 720 lines high, so any browser plays and seeks it.
    Proxy,
    /// A still from a video, as a JPEG: what a player shows before it
    /// plays.
    Poster,
    /// The waveform's peaks, as `audiowaveform` JSON ([`Peaks`]): what a
    /// player draws.
    Peaks,
    /// An audio file as AAC in an MP4, which every browser plays.
    Audio,
}

impl RenditionKind {
    /// Every kind, in the order a set is reported.
    pub const ALL: [Self; 4] = [Self::Proxy, Self::Poster, Self::Peaks, Self::Audio];

    /// Its name in a URL (`proxy`, `poster`, `peaks`, `audio`).
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::Poster => "poster",
            Self::Peaks => "peaks",
            Self::Audio => "audio",
        }
    }

    /// The content type it is served as.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Proxy => "video/mp4",
            Self::Poster => "image/jpeg",
            Self::Peaks => "application/json",
            Self::Audio => "audio/mp4",
        }
    }

    /// The kinds a source of `class` yields.
    #[must_use]
    pub const fn ladder(class: MediaClass) -> &'static [Self] {
        match class {
            MediaClass::Video => &[Self::Proxy, Self::Poster, Self::Peaks],
            MediaClass::Audio => &[Self::Peaks, Self::Audio],
            MediaClass::Other => &[],
        }
    }
}

impl fmt::Display for RenditionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// A word that names no [`RenditionKind`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no rendition is called `{0}`: proxy, poster, peaks or audio")]
pub struct UnknownKind(pub String);

impl FromStr for RenditionKind {
    type Err = UnknownKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.tag() == s)
            .ok_or_else(|| UnknownKind(s.to_owned()))
    }
}

/// What a source is, as the transcoder found it (never guessed from a
/// file's name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum MediaClass {
    /// It has a picture.
    Video,
    /// It has sound and no picture.
    Audio,
    /// Not media: nothing is derived from it.
    Other,
}

/// Where one rendition of a version is.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum RenditionState {
    /// Made: its bytes are `content`, `size` long.
    Ready { content: ContentId, size: u64 },
    /// Being made, or waiting its turn: ask again.
    Pending,
    /// The transcoder could not make it (a video with no sound has no
    /// peaks). Not tried again until the recipe changes.
    Failed { reason: String },
}

/// One rendition and where it is.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct RenditionStatus {
    pub kind: RenditionKind,
    pub state: RenditionState,
}

/// A version's renditions.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct RenditionSet {
    /// The version's bytes: what the renditions are keyed by.
    pub content: ContentId,
    /// What the source is; `None` until the transcoder has looked.
    pub class: Option<MediaClass>,
    /// One per kind its class yields (empty for [`MediaClass::Other`],
    /// and until it has been looked at).
    pub kinds: Vec<RenditionStatus>,
    /// Why the transcoder could not look at the source, when it could
    /// not (nothing is made of it until the server restarts).
    pub problem: Option<String>,
}

impl RenditionSet {
    /// The state of `kind`, if the source yields it.
    #[must_use]
    pub fn state(&self, kind: RenditionKind) -> Option<&RenditionState> {
        self.kinds.iter().find(|k| k.kind == kind).map(|k| &k.state)
    }

    /// Whether nothing is still being made.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.problem.is_some()
            || self.class.is_some()
                && self
                    .kinds
                    .iter()
                    .all(|k| !matches!(k.state, RenditionState::Pending))
    }
}

/// A read ticket: what a browser element that cannot send a header
/// (`<video>`, `<audio>`, `<img>`) appends to a byte URL as `?token=`.
///
/// Minted over the org's lane, where the gate knows the caller
/// ([`crate::FilesService::ticket`]); it reads `path` and everything
/// under it in one root, as the person who minted it, and only what
/// they may still read when it is used.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Ticket {
    /// Opaque: the `token` of a byte URL ([`crate::web::content_url`]).
    pub token: String,
    /// Microseconds since the Unix epoch after which it reads nothing.
    pub expires_us: i64,
}

/// Peaks per second of sound in a [`RenditionKind::Peaks`] rendition.
pub const PEAKS_PER_SECOND: u32 = 100;

/// A waveform's peaks in `audiowaveform`'s JSON format (version 2, one
/// channel, 16 bits): a `[min, max]` pair per `samples_per_pixel` samples,
/// flattened into `data` — what `peaks.js` and `wavesurfer.js` load as is.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Peaks {
    pub version: u32,
    pub channels: u32,
    pub sample_rate: u32,
    pub samples_per_pixel: u32,
    pub bits: u32,
    /// How many pairs `data` holds.
    pub length: u32,
    pub data: Vec<i16>,
}

impl Peaks {
    /// Reduce mono signed 16-bit samples at `sample_rate` to
    /// [`PEAKS_PER_SECOND`] pairs a second.
    #[must_use]
    pub fn from_samples(samples: &[i16], sample_rate: u32) -> Self {
        let mut reducer = PeakReducer::new(sample_rate);
        reducer.push(samples);
        reducer.finish()
    }

    /// [`Self::from_samples`] over raw little-endian 16-bit PCM (an odd
    /// trailing byte is dropped).
    #[must_use]
    pub fn from_pcm_s16le(pcm: &[u8], sample_rate: u32) -> Self {
        let mut reducer = PeakReducer::new(sample_rate);
        reducer.push_pcm_s16le(pcm);
        reducer.finish()
    }
}

/// [`Peaks`] made a run of samples at a time, so an hour of sound is
/// never held whole.
#[derive(Debug, Clone)]
pub struct PeakReducer {
    sample_rate: u32,
    window: u32,
    seen: u32,
    lo: i16,
    hi: i16,
    data: Vec<i16>,
}

impl PeakReducer {
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            window: (sample_rate / PEAKS_PER_SECOND).max(1),
            seen: 0,
            lo: i16::MAX,
            hi: i16::MIN,
            data: Vec::new(),
        }
    }

    pub fn push(&mut self, samples: &[i16]) {
        for &s in samples {
            self.lo = self.lo.min(s);
            self.hi = self.hi.max(s);
            self.seen = self.seen.saturating_add(1);
            if self.seen == self.window {
                self.close();
            }
        }
    }

    /// Raw little-endian 16-bit PCM; a run should hold whole samples.
    pub fn push_pcm_s16le(&mut self, pcm: &[u8]) {
        let samples: Vec<i16> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        self.push(&samples);
    }

    fn close(&mut self) {
        self.data.push(self.lo);
        self.data.push(self.hi);
        self.seen = 0;
        self.lo = i16::MAX;
        self.hi = i16::MIN;
    }

    /// The peaks, a short last window included.
    #[must_use]
    pub fn finish(mut self) -> Peaks {
        if self.seen > 0 {
            self.close();
        }
        Peaks {
            version: 2,
            channels: 1,
            sample_rate: self.sample_rate,
            samples_per_pixel: self.window,
            bits: 16,
            length: u32::try_from(self.data.len() / 2).unwrap_or(u32::MAX),
            data: self.data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_their_tags() {
        for kind in RenditionKind::ALL {
            assert_eq!(kind.tag().parse::<RenditionKind>(), Ok(kind));
        }
        assert!("thumbnail".parse::<RenditionKind>().is_err());
    }

    #[test]
    fn a_video_yields_a_proxy_a_poster_and_peaks_and_audio_peaks_and_a_transcode() {
        assert_eq!(
            RenditionKind::ladder(MediaClass::Video),
            &[
                RenditionKind::Proxy,
                RenditionKind::Poster,
                RenditionKind::Peaks
            ]
        );
        assert_eq!(
            RenditionKind::ladder(MediaClass::Audio),
            &[RenditionKind::Peaks, RenditionKind::Audio]
        );
        assert!(RenditionKind::ladder(MediaClass::Other).is_empty());
    }

    #[test]
    fn peaks_are_a_min_and_max_per_window() {
        // 8000 Hz: 80 samples a peak. Two windows and a short third.
        let mut samples = vec![0_i16; 200];
        samples[3] = -900;
        samples[10] = 1200;
        samples[100] = 50;
        samples[199] = -7;
        let peaks = Peaks::from_samples(&samples, 8000);
        assert_eq!(peaks.samples_per_pixel, 80);
        assert_eq!(peaks.length, 3);
        assert_eq!(peaks.data, vec![-900, 1200, 0, 50, -7, 0]);

        let pcm: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(Peaks::from_pcm_s16le(&pcm, 8000), peaks);
    }
}
