//! An Ogg Vorbis stream decoded a packet at a time, from bytes: what the
//! web app's own player decodes (`app::decoded`), kept apart from the
//! browser so it is tested natively.
//!
//! The bytes are the whole file (a few MB); nothing decodes it whole. The
//! player seeks to a frame and decodes forward from exactly there, a
//! second or two ahead of what is heard. Pure Rust: symphonia's Ogg
//! reader and Vorbis decoder.

use std::io::Cursor;

use symphonia_codec_vorbis::VorbisDecoder;
use symphonia_core::audio::SampleBuffer;
use symphonia_core::codecs::{Decoder as _, DecoderOptions};
use symphonia_core::errors::Error as SymphoniaError;
use symphonia_core::formats::{FormatOptions, FormatReader as _, SeekMode, SeekTo};
use symphonia_core::io::MediaSourceStream;
use symphonia_format_ogg::OggReader;

/// How far before a sought frame the reader goes: Vorbis's largest block.
const PREROLL: u64 = 8192;

/// One Ogg Vorbis stream, decoded forward from where it was last sought.
pub struct Stream {
    reader: OggReader,
    decoder: VorbisDecoder,
    track: u32,
    rate: u32,
    channels: usize,
    frames: Option<u64>,
    /// The frame the last seek asked for: decoded frames before it (the
    /// ones from the page the seek landed on) are discarded.
    from: u64,
}

impl Stream {
    /// The stream in `bytes` (the whole file, as fetched).
    ///
    /// # Errors
    ///
    /// Not Ogg, not Vorbis, or no rate or channels.
    pub fn open(bytes: Vec<u8>) -> Result<Self, String> {
        let source = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
        let reader = OggReader::try_new(source, &FormatOptions::default())
            .map_err(|e| format!("not an Ogg stream: {e}"))?;
        let track = reader
            .default_track()
            .ok_or("the Ogg stream has no track")?;
        let params = track.codec_params.clone();
        let id = track.id;
        let decoder = VorbisDecoder::try_new(&params, &DecoderOptions::default())
            .map_err(|e| format!("not Vorbis: {e}"))?;
        let rate = params
            .sample_rate
            .filter(|r| *r > 0)
            .ok_or("no sample rate")?;
        let channels = params
            .channels
            .map_or(0, symphonia_core::audio::Channels::count);
        if channels == 0 {
            return Err("no channels".to_owned());
        }
        Ok(Self {
            reader,
            decoder,
            track: id,
            rate,
            channels,
            frames: params.n_frames,
            from: 0,
        })
    }

    /// To `frame`: the next [`Self::decode`] starts exactly there.
    ///
    /// # Errors
    ///
    /// The stream cannot be sought.
    pub fn seek(&mut self, frame: u64) -> Result<(), String> {
        // A packet's frames need the packet before it (Vorbis overlaps
        // its windows), and a decoder that was just reset has none: so
        // the reader goes back a block more than asked, and what is
        // decoded before `frame` is discarded.
        let to = SeekTo::TimeStamp {
            ts: frame.saturating_sub(PREROLL),
            track_id: self.track,
        };
        self.reader
            .seek(SeekMode::Accurate, to)
            .map_err(|e| format!("seek: {e}"))?;
        self.decoder.reset();
        self.from = frame;
        Ok(())
    }

    /// Its sample rate.
    #[must_use]
    pub const fn rate(&self) -> u32 {
        self.rate
    }

    /// Its length in frames, where the container says.
    #[must_use]
    pub const fn frames(&self) -> Option<u64> {
        self.frames
    }

    /// The next packet's frames, appended to `out` as interleaved stereo
    /// (a mono stream on both sides, the first two channels of a wider
    /// one). False at the stream's end.
    ///
    /// # Errors
    ///
    /// The stream cannot be read or decoded.
    pub fn decode(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(p) => p,
                // The end of the bytes is the end of the song.
                Err(SymphoniaError::IoError(_) | SymphoniaError::ResetRequired) => {
                    return Ok(false);
                }
                Err(e) => return Err(format!("read: {e}")),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let ends_at = packet.ts().saturating_add(packet.dur());
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                // A damaged packet is skipped, as a player does.
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(format!("decode: {e}")),
            };
            if decoded.frames() == 0 {
                continue;
            }
            let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            samples.copy_interleaved_ref(decoded);
            let all = samples.samples();
            let frames = all.len() / self.channels;
            // Where these frames are in the song: they END where the
            // packet does. The first packet after a seek has no packet
            // before it to overlap with and comes out short, at its end.
            let first = (ends_at).saturating_sub(frames as u64);
            let from = usize::try_from(self.from.saturating_sub(first))
                .unwrap_or(usize::MAX)
                .min(frames);
            for frame in all.chunks_exact(self.channels).skip(from) {
                let left = frame.first().copied().unwrap_or(0.0);
                out.push(left);
                out.push(frame.get(1).copied().unwrap_or(left));
            }
            if from < frames {
                return Ok(true);
            }
        }
    }
}

/// Stereo at one rate to another, a block at a time, by straight lines
/// between samples: its state carries over, so blocks join with no step.
/// Only for a stream whose rate is not the context's.
#[derive(Default)]
pub struct Resampler {
    /// Where the next output frame is, in input frames from the start of
    /// the next block (below zero: between the last block's end and it).
    at: f64,
    last: [f32; 2],
}

impl Resampler {
    pub fn run(&mut self, input: &[f32], from: u32, to: u32, out: &mut Vec<f32>) {
        let step = f64::from(from) / f64::from(to);
        let frames = input.len() / 2;
        let sample = |i: isize, c: usize| -> f32 {
            usize::try_from(i).map_or(self.last[c], |i| {
                input.get(i * 2 + c).copied().unwrap_or(0.0)
            })
        };
        while self.at < frames as f64 - 1.0 {
            let i = self.at.floor();
            let t = (self.at - i) as f32;
            #[allow(clippy::cast_possible_truncation)]
            let i = i as isize;
            for c in 0..2 {
                let (a, b) = (sample(i, c), sample(i + 1, c));
                out.push(a + (b - a) * t);
            }
            self.at += step;
        }
        self.at -= frames as f64;
        if frames > 0 {
            self.last = [
                input.get(frames * 2 - 2).copied().unwrap_or(0.0),
                input.get(frames * 2 - 1).copied().unwrap_or(0.0),
            ];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three seconds of stereo at 48 kHz: 440 Hz on the left, 660 Hz on
    /// the right (`oggenc -q 2` of a generated tone).
    const TONE: &[u8] = include_bytes!("../test/fixtures/tone.ogg");

    fn all(stream: &mut Stream) -> Vec<f32> {
        let mut out = Vec::new();
        while stream.decode(&mut out).unwrap() {}
        out
    }

    /// How often `channel` crosses zero upward, a second: its pitch.
    fn pitch(pcm: &[f32], channel: usize, rate: u32) -> f64 {
        let side: Vec<f32> = pcm.chunks_exact(2).map(|f| f[channel]).collect();
        let ups = side
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        ups as f64 / (side.len() as f64 / f64::from(rate))
    }

    #[test]
    fn it_opens_and_decodes_the_whole_song() {
        let mut stream = Stream::open(TONE.to_vec()).unwrap();
        assert_eq!(stream.rate(), 48_000);
        // The tone is 144,000 frames; the container rounds up to its last
        // block (under 1,024 frames more).
        let frames = stream.frames().unwrap();
        assert!((144_000..145_024).contains(&frames), "{frames} frames");
        let pcm = all(&mut stream);
        assert_eq!((pcm.len() / 2) as u64, frames, "every frame, as stereo");
        assert!((pitch(&pcm, 0, 48_000) - 440.0).abs() < 3.0);
        assert!((pitch(&pcm, 1, 48_000) - 660.0).abs() < 3.0);
    }

    #[test]
    fn a_seek_lands_on_the_frame_asked_for() {
        // From frame 72,000 the stream is the same samples the whole
        // decode has from there: the frames between the page the seek
        // landed on and the target are discarded, none kept, none lost.
        let whole = all(&mut Stream::open(TONE.to_vec()).unwrap());
        for at in [0_u64, 1, 4_800, 72_000, 100_001, 143_000] {
            let mut stream = Stream::open(TONE.to_vec()).unwrap();
            stream.seek(at).unwrap();
            let rest = all(&mut stream);
            let want = &whole[usize::try_from(at).unwrap() * 2..];
            assert_eq!(rest.len(), want.len(), "from frame {at}");
            // Vorbis overlaps windows: the first packet after a seek has
            // no history, so its first frames differ a little; after it
            // the samples are the same.
            let settled = 4096.min(rest.len());
            let far = rest[settled..]
                .iter()
                .zip(&want[settled..])
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            assert!(far < 1e-3, "from frame {at}: off by {far}");
        }
    }

    #[test]
    fn a_seek_back_plays_again() {
        let mut stream = Stream::open(TONE.to_vec()).unwrap();
        let first = all(&mut stream);
        stream.seek(0).unwrap();
        assert_eq!(all(&mut stream).len(), first.len());
    }

    #[test]
    fn what_is_not_ogg_vorbis_is_refused() {
        assert!(Stream::open(b"ID3 not an ogg".to_vec()).is_err());
        assert!(Stream::open(Vec::new()).is_err());
    }

    #[test]
    fn resampled_blocks_join_without_a_step() {
        // A ramp at 44.1 kHz in two blocks comes out a ramp at 48 kHz:
        // no sample steps more than the ramp's own slope allows.
        let ramp: Vec<f32> = (0..2000).flat_map(|i| [i as f32, i as f32]).collect();
        let (a, b) = ramp.split_at(2000);
        let mut r = Resampler::default();
        let mut out = Vec::new();
        r.run(a, 44_100, 48_000, &mut out);
        r.run(b, 44_100, 48_000, &mut out);
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        assert!(left.len() > 2000);
        for pair in left.windows(2) {
            let step = pair[1] - pair[0];
            assert!((0.0..=1.0).contains(&step), "a step of {step}");
        }
    }
}
