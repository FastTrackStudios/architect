//! What a native app hears (#589, feature `native-audio`): the deck's
//! active source, decoded in the app and played on the device's output.
//!
//! The web app's clock is its `<audio>` element; a native app has none,
//! and iOS decodes no Ogg Vorbis of its own, so the app decodes the file
//! itself (Symphonia: Ogg Vorbis, MP3, AAC in MP4, FLAC, WAV) and hands
//! the samples to the device (cpal: CoreAudio on macOS and iOS). One
//! thread reads and decodes into a ring the device's callback drains;
//! [`NativeClock`] is the [`Clock`] over it, and [`start`] follows the
//! deck with it, as `apps/web/src/app/audio.rs` does with its element.
//!
//! An item's media is a vault path; the app says which file that is
//! ([`Resolve`]: its open vault's, or none, and then the deck's own
//! silent clock goes on). The device is opened at the first play, never
//! at a load: opening a set does not stop what another app plays.
//!
//! On iOS ([`ios`]) the audio session is `playback` (heard with the
//! ringer off and with the screen locked, with `UIBackgroundModes`
//! `audio` in the app's Info.plist), and the lock screen shows the item,
//! its art and where it is, and sends play, pause, next, previous and its
//! scrubber to the deck as the app's own presses.

#[cfg(target_os = "ios")]
mod ios;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde_json::Value;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::clock::{Clock, Follower, Reading};

/// The file an item's media (or art) at this vault path is, if the app
/// has it on this device.
pub type Resolve = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

/// What the decoding thread, the device's callback and the clock share.
#[derive(Default)]
struct Shared {
    /// The deck plays and the device runs: the callback drains the ring.
    playing: AtomicBool,
    /// Device frames heard since the ring was made.
    played: AtomicU64,
    /// Seconds into the media the ring starts at (`f64` bits).
    base: AtomicU64,
    /// The device's sample rate; 0 before it is open.
    rate: AtomicU32,
    /// The media's length in seconds (`f64` bits; NaN while unknown).
    duration: AtomicU64,
    /// The decoder reached the file's end.
    eof: AtomicBool,
    /// The file's end was heard: the ring ran dry after `eof`.
    drained: AtomicBool,
    /// The ring ran dry before the file's end.
    starved: AtomicBool,
    /// The file could not be opened or decoded, or the device not opened.
    failed: AtomicBool,
    /// Loads and seeks the clock asked for, and those the thread has done:
    /// between the two the ring's time is not the item's.
    asked: AtomicU64,
    done: AtomicU64,
    /// The samples to hear, interleaved in the device's layout. The
    /// callback only tries the lock: the thread holds it to swap rings.
    ring: Mutex<Option<rtrb::Consumer<f32>>>,
}

impl Shared {
    fn seconds(bits: &AtomicU64) -> f64 {
        f64::from_bits(bits.load(Ordering::Relaxed))
    }

    fn settled(&self) -> bool {
        self.asked.load(Ordering::Acquire) == self.done.load(Ordering::Acquire)
    }
}

enum Cmd {
    Load(Option<PathBuf>, u64),
    Seek(f64, u64),
    Play,
    Pause,
}

/// The device's output, as a [`Clock`].
pub struct NativeClock {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
    resolve: Resolve,
    loaded: bool,
}

impl NativeClock {
    /// A clock over a decoding thread of its own, with nothing loaded.
    #[must_use]
    pub fn new(resolve: Resolve) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        shared.duration.store(f64::NAN.to_bits(), Ordering::Relaxed);
        let theirs = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("playhead-audio".to_owned())
            .spawn(move || Engine::new(theirs).run(&rx));
        if spawned.is_err() {
            shared.failed.store(true, Ordering::Relaxed);
        }
        Self {
            tx,
            shared,
            resolve,
            loaded: false,
        }
    }

    fn ask(&self, cmd: impl FnOnce(u64) -> Cmd) {
        let n = self.shared.asked.fetch_add(1, Ordering::AcqRel) + 1;
        if self.tx.send(cmd(n)).is_err() {
            self.shared.failed.store(true, Ordering::Relaxed);
        }
    }
}

impl Clock for NativeClock {
    fn kind(&self) -> &'static str {
        "media"
    }

    fn load(&mut self, media: Option<&str>) {
        let file = media
            .filter(|p| crate::model::vault_path(p))
            .and_then(|p| (self.resolve)(p));
        self.loaded = file.is_some();
        // The last item's end is not this one's.
        self.shared.eof.store(false, Ordering::Relaxed);
        self.shared.drained.store(false, Ordering::Relaxed);
        self.shared.failed.store(false, Ordering::Relaxed);
        self.shared
            .duration
            .store(f64::NAN.to_bits(), Ordering::Relaxed);
        self.ask(|n| Cmd::Load(file, n));
    }

    fn play(&mut self) {
        if self.loaded {
            let _ = self.tx.send(Cmd::Play);
        }
    }

    fn pause(&mut self) {
        let _ = self.tx.send(Cmd::Pause);
    }

    fn seek(&mut self, t: f64) {
        if self.loaded && t.is_finite() {
            self.shared.drained.store(false, Ordering::Relaxed);
            self.ask(|n| Cmd::Seek(t.max(0.0), n));
        }
    }

    fn read(&self) -> Option<Reading> {
        let s = &self.shared;
        // No file for the media here, or one that cannot be played: not
        // heard, the deck's clock goes on.
        if !self.loaded || s.failed.load(Ordering::Relaxed) {
            return Some(Reading {
                failed: true,
                ..Reading::default()
            });
        }
        let settled = s.settled();
        let rate = s.rate.load(Ordering::Relaxed);
        let heard = if rate == 0 {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            let frames = s.played.load(Ordering::Relaxed) as f64;
            frames / f64::from(rate)
        };
        let duration = Shared::seconds(&s.duration);
        let drained = settled && s.drained.load(Ordering::Relaxed);
        Some(Reading {
            time: Shared::seconds(&s.base) + heard,
            duration: (duration.is_finite() && duration > 0.0).then_some(duration),
            moving: settled
                && rate != 0
                && s.playing.load(Ordering::Relaxed)
                && !s.starved.load(Ordering::Relaxed)
                && !drained,
            ended: drained,
            failed: false,
            resolved: true,
        })
    }
}

/// A file being decoded.
struct Source {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    rate: u32,
    channels: usize,
    samples: Option<SampleBuffer<f32>>,
    /// Linear resampling to the device's rate: where the next output
    /// frame is, in input frames from the current packet's first (may be
    /// in `[-1, 0)`: between the last packet's last frame and it).
    pos: f64,
    last: Vec<f32>,
}

impl Source {
    fn open(path: &Path) -> Option<(Self, Option<f64>)> {
        let file = std::fs::File::open(path).ok()?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .ok()?
            .format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
        let params = &track.codec_params;
        let rate = params.sample_rate.filter(|r| *r > 0)?;
        #[allow(clippy::cast_precision_loss)]
        let duration = params.n_frames.map(|n| n as f64 / f64::from(rate));
        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .ok()?;
        let id = track.id;
        Some((
            Self {
                format,
                decoder,
                track: id,
                rate,
                channels: 0,
                samples: None,
                pos: 0.0,
                last: Vec::new(),
            },
            duration,
        ))
    }

    /// To `t` seconds: where it really is now (a packet's start at or
    /// before `t`), in seconds.
    fn seek(&mut self, t: f64) -> f64 {
        let to = SeekTo::Time {
            time: Time::from(t),
            track_id: Some(self.track),
        };
        let at = match self.format.seek(SeekMode::Accurate, to) {
            #[allow(clippy::cast_precision_loss)]
            Ok(seeked) => seeked.actual_ts as f64 / f64::from(self.rate),
            Err(_) => t,
        };
        self.decoder.reset();
        self.pos = 0.0;
        self.last.clear();
        at
    }

    /// The next packet's samples, in the device's rate and layout, onto
    /// `out`. `Ok(false)` at the file's end.
    fn next(&mut self, out: &mut Vec<f32>, device: (u32, usize)) -> Result<bool, ()> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(DecodeError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(DecodeError::ResetRequired) => return Ok(false),
                Err(_) => return Err(()),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                // A packet it cannot read is passed over, not the file.
                Err(DecodeError::DecodeError(_)) => continue,
                Err(_) => return Err(()),
            };
            let spec = *decoded.spec();
            let channels = spec.channels.count();
            if channels == 0 {
                continue;
            }
            let frames = decoded.capacity() as u64;
            let fits = self.samples.as_ref().is_some_and(|b| {
                self.channels == channels && b.capacity() >= decoded.capacity() * channels
            });
            if !fits {
                self.samples = Some(SampleBuffer::new(frames, spec));
                self.channels = channels;
            }
            let Some(buffer) = self.samples.as_mut() else {
                return Err(());
            };
            buffer.copy_interleaved_ref(decoded);
            let (pos, last) = convert(
                buffer.samples(),
                (self.rate, channels),
                device,
                (self.pos, &self.last),
                out,
            );
            self.pos = pos;
            self.last = last;
            return Ok(true);
        }
    }
}

/// One output sample for device channel `c` of a frame with `from`
/// channels: mono on both sides of a stereo device, a stereo pair as it
/// is, a mono device the pair's mean, nothing in channels past the pair.
fn mapped(frame: &[f32], c: usize, to: usize) -> f32 {
    let left = frame.first().copied().unwrap_or(0.0);
    let right = frame.get(1).copied().unwrap_or(left);
    match (to, c) {
        (1, _) => (left + right) * 0.5,
        (_, 0) => left,
        (_, 1) => right,
        _ => 0.0,
    }
}

/// `input` (interleaved, `from` = rate and channels) as the device hears
/// it (`to`), appended to `out`; linear between frames when the rates
/// differ. `carry` is where the last call stopped and its last frame; the
/// same is returned for the next.
fn convert(
    input: &[f32],
    from: (u32, usize),
    to: (u32, usize),
    carry: (f64, &[f32]),
    out: &mut Vec<f32>,
) -> (f64, Vec<f32>) {
    let (rate, channels) = from;
    let (device_rate, device_channels) = to;
    let frames = input.len() / channels.max(1);
    let frame = |i: usize| input.get(i * channels..(i + 1) * channels).unwrap_or(&[]);
    if frames == 0 {
        return (carry.0, carry.1.to_vec());
    }
    let last = frame(frames - 1).to_vec();
    if rate == device_rate {
        for i in 0..frames {
            let f = frame(i);
            out.extend((0..device_channels).map(|c| mapped(f, c, device_channels)));
        }
        return (0.0, last);
    }
    let step = f64::from(rate) / f64::from(device_rate);
    let mut pos = carry.0;
    #[allow(clippy::cast_precision_loss)]
    let end = (frames - 1) as f64;
    let silence = vec![0.0; channels];
    let before: &[f32] = if carry.1.len() == channels {
        carry.1
    } else {
        &silence
    };
    while pos <= end {
        let floor = pos.floor();
        #[allow(clippy::cast_possible_truncation)]
        let mix = (pos - floor) as f32;
        // `floor` is -1 between the last call's last frame and this one's first.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (a, b) = if floor < 0.0 {
            (before, frame(0))
        } else {
            let i = floor as usize;
            (frame(i), frame((i + 1).min(frames - 1)))
        };
        out.extend((0..device_channels).map(|c| {
            let (x, y) = (mapped(a, c, device_channels), mapped(b, c, device_channels));
            (y - x).mul_add(mix, x)
        }));
        pos += step;
    }
    #[allow(clippy::cast_precision_loss)]
    let next = pos - frames as f64;
    (next, last)
}

/// The device's output stream, once the first play opened it.
struct Output {
    stream: cpal::Stream,
    rate: u32,
    channels: usize,
}

impl Output {
    fn open(shared: &Arc<Shared>) -> Option<Self> {
        #[cfg(target_os = "ios")]
        ios::activate_session();
        let device = cpal::default_host().default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let config: cpal::StreamConfig = supported.config();
        let rate = config.sample_rate;
        let channels = usize::from(config.channels);
        if rate == 0 || channels == 0 {
            return None;
        }
        let theirs = Arc::clone(shared);
        let stream = device
            .build_output_stream(
                config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| fill(out, channels, &theirs),
                |error| tracing::warn!(%error, "playhead.audio.stream_error"),
                None,
            )
            .ok()?;
        shared.rate.store(rate, Ordering::Relaxed);
        Some(Self {
            stream,
            rate,
            channels,
        })
    }
}

/// The device's callback: the ring into `out`, silence where it has none.
fn fill(out: &mut [f32], channels: usize, shared: &Shared) {
    let mut written = 0;
    if shared.playing.load(Ordering::Relaxed)
        && let Ok(mut ring) = shared.ring.try_lock()
        && let Some(ring) = ring.as_mut()
    {
        let want = out.len().min(ring.slots());
        let want = want - want % channels;
        if let Ok(chunk) = ring.read_chunk(want) {
            let (a, b) = chunk.as_slices();
            if let Some(dst) = out.get_mut(..a.len()) {
                dst.copy_from_slice(a);
            }
            if let Some(dst) = out.get_mut(a.len()..a.len() + b.len()) {
                dst.copy_from_slice(b);
            }
            chunk.commit_all();
            written = want;
        }
        shared
            .played
            .fetch_add((written / channels) as u64, Ordering::Relaxed);
        let dry = written < out.len();
        let eof = shared.eof.load(Ordering::Relaxed);
        shared.starved.store(dry && !eof, Ordering::Relaxed);
        if dry && eof && ring.is_empty() {
            shared.drained.store(true, Ordering::Relaxed);
        }
    }
    if let Some(rest) = out.get_mut(written..) {
        rest.fill(0.0);
    }
}

/// The decoding thread: the file, the ring's writing end and the device.
struct Engine {
    shared: Arc<Shared>,
    source: Option<Source>,
    output: Option<Output>,
    ring: Option<rtrb::Producer<f32>>,
    /// Converted samples the ring had no room for yet.
    pending: Vec<f32>,
    sent: usize,
    wants_play: bool,
}

impl Engine {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            source: None,
            output: None,
            ring: None,
            pending: Vec::new(),
            sent: 0,
            wants_play: false,
        }
    }

    fn run(mut self, rx: &Receiver<Cmd>) {
        loop {
            // While there is decoding to do, commands are taken between
            // packets; otherwise it sleeps on them.
            let cmd = if self.busy() {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            } else {
                match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            match cmd {
                Some(Cmd::Load(file, n)) => {
                    self.load(file.as_deref());
                    self.shared.done.store(n, Ordering::Release);
                }
                Some(Cmd::Seek(t, n)) => {
                    self.seek(t);
                    self.shared.done.store(n, Ordering::Release);
                }
                Some(Cmd::Play) => self.play(),
                Some(Cmd::Pause) => self.pause(),
                None => {}
            }
            if !self.pump() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// Whether there is a file left to decode into an open device.
    fn busy(&self) -> bool {
        self.source.is_some() && self.output.is_some() && !self.shared.eof.load(Ordering::Relaxed)
            || self.sent < self.pending.len()
    }

    /// A new ring, starting `at` seconds into the media.
    fn restart(&mut self, at: f64) {
        self.pending.clear();
        self.sent = 0;
        self.shared.eof.store(false, Ordering::Relaxed);
        self.shared.starved.store(false, Ordering::Relaxed);
        let ring = self.output.as_ref().map(|o| {
            // Two seconds of the device's audio.
            rtrb::RingBuffer::new(o.rate as usize * o.channels * 2)
        });
        let (producer, consumer) = match ring {
            Some((p, c)) => (Some(p), Some(c)),
            None => (None, None),
        };
        let mut theirs = self
            .shared
            .ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *theirs = consumer;
        self.shared.played.store(0, Ordering::Relaxed);
        self.shared.base.store(at.to_bits(), Ordering::Relaxed);
        self.shared.drained.store(false, Ordering::Relaxed);
        drop(theirs);
        self.ring = producer;
    }

    fn load(&mut self, file: Option<&Path>) {
        self.source = None;
        let opened = file.map(Source::open);
        match opened {
            Some(Some((source, duration))) => {
                self.shared
                    .duration
                    .store(duration.unwrap_or(f64::NAN).to_bits(), Ordering::Relaxed);
                self.source = Some(source);
            }
            Some(None) => self.shared.failed.store(true, Ordering::Relaxed),
            None => {}
        }
        self.restart(0.0);
    }

    fn seek(&mut self, t: f64) {
        let at = self.source.as_mut().map_or(t, |s| s.seek(t));
        self.restart(at);
    }

    fn play(&mut self) {
        self.wants_play = true;
        if self.output.is_none() {
            self.output = Output::open(&self.shared);
            if self.output.is_none() {
                self.shared.failed.store(true, Ordering::Relaxed);
                return;
            }
            // The ring is the device's size: made now that it is known,
            // where the deck already is.
            let at = Shared::seconds(&self.shared.base);
            self.restart(at);
        }
        if let Some(o) = &self.output {
            if o.stream.play().is_err() {
                self.shared.failed.store(true, Ordering::Relaxed);
                return;
            }
            self.shared.playing.store(true, Ordering::Relaxed);
        }
    }

    fn pause(&mut self) {
        self.wants_play = false;
        self.shared.playing.store(false, Ordering::Relaxed);
        if let Some(o) = &self.output {
            let _ = o.stream.pause();
        }
    }

    /// Decoded samples into the ring: whether it did anything.
    fn pump(&mut self) -> bool {
        let (Some(output), Some(ring)) = (&self.output, self.ring.as_mut()) else {
            return false;
        };
        if self.sent < self.pending.len() {
            let room = ring.slots();
            let rest = self.pending.get(self.sent..).unwrap_or(&[]);
            let n = room.min(rest.len());
            if n == 0 {
                return false;
            }
            if let Ok(mut chunk) = ring.write_chunk_uninit(n) {
                let (a, b) = chunk.as_mut_slices();
                let (first, second) = rest.split_at(a.len().min(n));
                for (dst, src) in a.iter_mut().zip(first) {
                    dst.write(*src);
                }
                for (dst, src) in b.iter_mut().zip(second) {
                    dst.write(*src);
                }
                // SAFETY: the two slices hold `n` slots between them and
                // each was written above (`first` and `second` are `n`
                // samples, split at the first slice's length).
                unsafe { chunk.commit_all() };
            }
            self.sent += n;
            return true;
        }
        if self.shared.eof.load(Ordering::Relaxed) {
            return false;
        }
        let Some(source) = self.source.as_mut() else {
            return false;
        };
        self.pending.clear();
        self.sent = 0;
        match source.next(&mut self.pending, (output.rate, output.channels)) {
            Ok(true) => {}
            Ok(false) => self.shared.eof.store(true, Ordering::Relaxed),
            Err(()) => {
                self.shared.eof.store(true, Ordering::Relaxed);
                self.shared.failed.store(true, Ordering::Relaxed);
            }
        }
        true
    }
}

/// What follows the deck: the clock, its follower and what the lock
/// screen was last told.
struct Player {
    clock: NativeClock,
    follower: Follower,
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    resolve: Resolve,
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    shown: Option<Shown>,
}

/// What the lock screen was told, and when: it counts on from `time`.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    key: String,
    time: f64,
    at: f64,
    playing: bool,
}

impl Player {
    fn sync(&mut self) {
        let now = crate::now_ms();
        crate::with(|d| {
            self.follower.follow(d, &mut self.clock, now);
            self.follower.report(d, &self.clock, now);
        });
        #[cfg(target_os = "ios")]
        self.show(now);
    }

    /// The lock screen's card, when what it shows changed or its time is
    /// no longer where the card counts to.
    #[cfg(target_os = "ios")]
    fn show(&mut self, now: f64) {
        let state = crate::with(|d| d.active().and_then(|a| d.state(a, now)));
        let Some(s) = state else {
            if self.shown.take().is_some() {
                ios::now_playing(None);
            }
            return;
        };
        let text = |k: &str| {
            s.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let number = |k: &str| s.get(k).and_then(Value::as_f64);
        let playing = s.get("playing") == Some(&Value::Bool(true));
        let time = number("time").unwrap_or(0.0);
        let art = s
            .get("item")
            .and_then(|i| i.get("art"))
            .and_then(Value::as_str)
            .filter(|p| crate::model::vault_path(p))
            .and_then(|p| (self.resolve)(p));
        let key = format!(
            "{}|{}|{}|{:?}|{:?}",
            text("source"),
            s.get("index").map_or_else(String::new, Value::to_string),
            text("title"),
            number("duration"),
            art
        );
        let drifted = self.shown.as_ref().is_none_or(|was| {
            let expected = if was.playing {
                was.time + (now - was.at) / 1000.0
            } else {
                was.time
            };
            was.key != key || was.playing != playing || (expected - time).abs() > 1.0
        });
        if !drifted {
            return;
        }
        ios::now_playing(Some(&ios::Card {
            title: text("title"),
            artist: text("subtitle"),
            album: text("name"),
            duration: number("duration"),
            time,
            playing,
            art,
        }));
        self.shown = Some(Shown {
            key,
            time,
            at: now,
            playing,
        });
    }
}

/// Makes the deck heard on this device, once, at the app's start: a
/// thread follows the deck's active source with a [`NativeClock`], on
/// every change and a few times a second. On iOS the lock screen's
/// buttons are the app's own presses from then on.
pub fn start(resolve: Resolve) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let (wake, woken) = mpsc::channel::<()>();
    let player = Player {
        clock: NativeClock::new(Arc::clone(&resolve)),
        follower: Follower::default(),
        resolve,
        shown: None,
    };
    let spawned = std::thread::Builder::new()
        .name("playhead-follow".to_owned())
        .spawn(move || {
            let mut player = player;
            loop {
                match woken.recv_timeout(Duration::from_millis(200)) {
                    Ok(()) | Err(RecvTimeoutError::Timeout) => player.sync(),
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    if spawned.is_err() {
        tracing::warn!("playhead.audio.no_thread");
        return;
    }
    crate::with(|d| d.audible = true);
    // Every change, at once; on the follower's thread, never inside the
    // press that made it (the deck is not re-entered).
    let wake = Mutex::new(wake);
    crate::on_change("native.audio", move |_: &Value| {
        let _ = wake.lock().unwrap_or_else(PoisonError::into_inner).send(());
    });
    #[cfg(target_os = "ios")]
    ios::commands();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn the_same_rate_maps_channels_only() {
        let mut out = Vec::new();
        let (pos, last) = convert(
            &[0.1, 0.2, 0.3],
            (48_000, 1),
            (48_000, 2),
            (0.0, &[]),
            &mut out,
        );
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
        assert_eq!((pos, last), (0.0, vec![0.3]));
        let mut mono = Vec::new();
        convert(&[1.0, 0.0], (48_000, 2), (48_000, 1), (0.0, &[]), &mut mono);
        assert_eq!(mono, [0.5]);
    }

    #[test]
    fn another_rate_keeps_the_length_and_joins_packets() {
        // 44.1 kHz in two packets heard at 48 kHz: as many frames as one
        // second of the device, and a ramp stays a ramp over the join.
        #[allow(clippy::cast_precision_loss)]
        let ramp: Vec<f32> = (0..44_100).map(|i| i as f32).collect();
        let (first, second) = ramp.split_at(20_000);
        let mut out = Vec::new();
        let (pos, last) = convert(first, (44_100, 1), (48_000, 1), (0.0, &[]), &mut out);
        convert(second, (44_100, 1), (48_000, 1), (pos, &last), &mut out);
        assert!((out.len() as i64 - 48_000).abs() <= 1, "{}", out.len());
        for pair in out.windows(2) {
            let step = pair[1] - pair[0];
            assert!((step - 0.918_75).abs() < 0.01, "{step}");
        }
    }
}
