//! A recording the app decodes itself: Ogg Vorbis, heard on every browser.
//!
//! The app's `<audio>` element plays what the browser's own codecs play,
//! and an iPhone's Safari plays no Ogg Vorbis before iOS 18.4. A share
//! link's recordings are what the vault holds, so the app decodes them:
//! the file's bytes are fetched once (a few MB), decoded a second or two
//! ahead of the playhead in the page (symphonia's Ogg reader and Vorbis
//! decoder, pure Rust), and played through an AudioWorklet that plays the
//! blocks the page posts it.
//!
//! **Its clock is the audio thread's.** The worklet reports how many
//! frames it has played; where the song is, is where the seek put it plus
//! those frames. That is the time the deck reads ([`Decoded::read`]), so
//! a chart or lyrics following the playhead follow what is heard, to the
//! sample.
//!
//! **In the background.** A hidden page's timers are throttled to about
//! one a second. The page is asked for more by the worklet's own reports
//! (the audio thread's, never throttled) as well as by the app's timer,
//! and it keeps [`AHEAD_SECONDS`] queued.
//!
//! **Seeking** empties the worklet's queue under a new epoch (blocks and
//! reports of an older one are dropped on both sides), seeks the Ogg
//! stream to the page before the target and discards the frames up to it.
//!
//! Anything this cannot open (an Ogg that is not Vorbis, a fetch refused)
//! is reported as failed, and the caller plays it on the element instead
//! (`super::audio`).

use std::cell::RefCell;
use std::rc::Rc;

use architect_playhead::Reading;

use crate::ogg::{Resampler, Stream};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioContext, AudioWorkletNode};

/// How much is kept queued ahead of the audio thread, seconds: more than
/// a hidden page's one-a-second timer, little enough that a seek's flush
/// throws nothing much away.
const AHEAD_SECONDS: f64 = 2.5;
/// The most the page decodes in one go, frames: a phone decodes this in a
/// few milliseconds, so a press is never held up.
const STEP_FRAMES: usize = 16_384;
/// How long a context may take to start after a press, ms.
const START_MS: f64 = 2000.0;
/// The largest recording fetched to decode: an hour of Vorbis.
const MAX_BYTES: u32 = 96 * 1024 * 1024;

/// The worklet: plays the stereo blocks posted to it while it runs, and
/// reports the frames it has played about 47 times a second.
const PROCESSOR: &str = r"
class ArchitectDeck extends AudioWorkletProcessor {
  constructor() {
    super();
    this.blocks = []; this.at = 0; this.run = false; this.epoch = 0; this.played = 0; this.since = 0;
    this.port.onmessage = (e) => {
      const m = e.data;
      if (m.pcm) { if (m.epoch === this.epoch) this.blocks.push(m.pcm); }
      else if (m.flush !== undefined) { this.blocks = []; this.at = 0; this.epoch = m.flush; this.played = 0; }
      else if (m.run !== undefined) { this.run = m.run; }
    };
  }
  process(inputs, outputs) {
    const out = outputs[0];
    const left = out[0], right = out[1] || out[0];
    const n = left.length;
    let i = 0;
    if (this.run) {
      while (i < n && this.blocks.length) {
        const b = this.blocks[0];
        const frames = b.length >> 1;
        const take = Math.min(frames - this.at, n - i);
        for (let k = 0; k < take; k++) {
          const j = (this.at + k) << 1;
          left[i + k] = b[j];
          right[i + k] = b[j + 1];
        }
        i += take;
        this.at += take;
        if (this.at >= frames) { this.blocks.shift(); this.at = 0; }
      }
      this.played += i;
    }
    this.since += n;
    if (this.since >= 1024) {
      this.since = 0;
      this.port.postMessage({ epoch: this.epoch, played: this.played });
    }
    return true;
  }
}
registerProcessor('architect-deck', ArchitectDeck);
";

/// Whether the app decodes media at `url` itself: Ogg, by its name (a
/// Vorbis stream is what a vault's `.ogg` nearly always is; anything else
/// in the container fails to open and goes to the element).
#[must_use]
pub fn handles(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    path.ends_with(".ogg") || path.ends_with(".oga")
}

#[derive(Default)]
struct Inner {
    ctx: Option<AudioContext>,
    node: Option<AudioWorkletNode>,
    /// The worklet is being made (its module loads in a promise).
    starting: bool,
    url: Option<String>,
    /// Counts loads: a fetch that ends after another began is dropped.
    load: u64,
    stream: Option<Stream>,
    failed: bool,
    /// Play is wanted (the deck's transport).
    want: bool,
    /// Counts seeks and loads: the worklet's queue and its reports are of one.
    epoch: u32,
    /// Where this epoch began, seconds into the song.
    base: f64,
    /// Frames posted to the worklet and played by it, this epoch, at the
    /// context's rate.
    posted: u64,
    played: u64,
    /// The stream ran out: what is queued is the song's last.
    drained: bool,
    /// A seek asked before the bytes arrived.
    pending: Option<f64>,
    /// When play was last asked with the context not yet running, ms.
    asked: Option<f64>,
    resampler: Resampler,
}

impl Inner {
    fn ctx_rate(&self) -> f64 {
        self.ctx
            .as_ref()
            .map_or(48_000.0, |c| f64::from(c.sample_rate()))
    }

    fn post(&self, message: &JsValue) {
        if let Some(port) = self.node.as_ref().and_then(|n| n.port().ok()) {
            let _ = port.post_message(message);
        }
    }

    fn say(&self, key: &str, value: &JsValue) {
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&message, &key.into(), value);
        self.post(&message);
    }

    /// A new epoch at `t`: the stream sought there, the worklet's queue
    /// emptied.
    fn restart(&mut self, t: f64) {
        self.epoch = self.epoch.wrapping_add(1);
        self.base = t.max(0.0);
        self.posted = 0;
        self.played = 0;
        self.drained = false;
        self.resampler = Resampler::default();
        if let Some(stream) = self.stream.as_mut() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let frame = (self.base * f64::from(stream.rate())).round() as u64;
            if let Err(why) = stream.seek(frame) {
                self.fail(&why);
                return;
            }
        }
        self.say("flush", &JsValue::from(self.epoch));
        self.say("run", &JsValue::from(self.want));
    }

    fn fail(&mut self, why: &str) {
        // One line, with the shape and never the path: the element takes over.
        web_sys::console::warn_1(&format!("architect: a recording did not decode here ({why}); the browser's own player has it").into());
        self.failed = true;
        self.stream = None;
        self.say("run", &JsValue::FALSE);
    }

    /// Decodes and posts until [`AHEAD_SECONDS`] are queued, at most
    /// [`STEP_FRAMES`] a call (the next report asks again).
    fn feed(&mut self) {
        if self.node.is_none() || self.drained || self.failed {
            return;
        }
        let rate = self.ctx_rate();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ahead = (AHEAD_SECONDS * rate) as u64;
        if self.posted.saturating_sub(self.played) >= ahead {
            return;
        }
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let mut block = Vec::with_capacity(STEP_FRAMES * 2);
        while block.len() < STEP_FRAMES * 2 {
            match stream.decode(&mut block) {
                Ok(true) => {}
                Ok(false) => {
                    self.drained = true;
                    break;
                }
                Err(why) => {
                    self.fail(&why);
                    return;
                }
            }
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let to = rate.round() as u32;
        let from = stream.rate();
        let pcm = if from == to {
            block
        } else {
            let mut out = Vec::with_capacity(block.len());
            self.resampler.run(&block, from, to, &mut out);
            out
        };
        if pcm.is_empty() {
            return;
        }
        let message = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &message,
            &"pcm".into(),
            &js_sys::Float32Array::from(pcm.as_slice()),
        );
        let _ = js_sys::Reflect::set(&message, &"epoch".into(), &JsValue::from(self.epoch));
        self.post(&message);
        self.posted += (pcm.len() / 2) as u64;
    }

    fn time(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let played = self.played as f64 / self.ctx_rate();
        self.base + played
    }

    fn duration(&self) -> Option<f64> {
        let s = self.stream.as_ref()?;
        #[allow(clippy::cast_precision_loss)]
        s.frames().map(|f| f as f64 / f64::from(s.rate()))
    }
}

/// The app's own player for a recording it decodes ([`handles`]).
#[derive(Clone)]
pub struct Decoded {
    inner: Rc<RefCell<Inner>>,
    /// Told when something the deck should hear of happened outside a
    /// press: the bytes arrived, the song ended.
    heard: Rc<dyn Fn()>,
}

impl Decoded {
    pub fn new(heard: impl Fn() + 'static) -> Self {
        Self {
            inner: Rc::default(),
            heard: Rc::new(heard),
        }
    }

    /// The recording at this origin's `url` from now on (none: nothing).
    pub fn load(&self, url: Option<&str>) {
        let load = {
            let mut i = self.inner.borrow_mut();
            i.load += 1;
            i.url = url.map(str::to_owned);
            i.stream = None;
            i.failed = false;
            i.pending = None;
            i.asked = None;
            i.restart(0.0);
            i.load
        };
        let Some(url) = url.map(str::to_owned) else {
            return;
        };
        let this = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let opened = match fetch(&url).await {
                Ok(bytes) => Stream::open(bytes),
                Err(why) => Err(why),
            };
            {
                let mut i = this.inner.borrow_mut();
                if i.load != load {
                    return;
                }
                match opened {
                    Ok(stream) => {
                        i.stream = Some(stream);
                        let at = i.pending.take().unwrap_or(0.0);
                        i.restart(at);
                        i.feed();
                    }
                    Err(why) => i.fail(&why),
                }
            }
            (this.heard)();
        });
    }

    /// Start. Called inside the press that asked, so the browser lets the
    /// context run.
    pub fn play(&self) {
        let start = {
            let mut i = self.inner.borrow_mut();
            i.want = true;
            if i.ctx.is_none() {
                // The rate most recordings are at, so nothing is resampled
                // in the page; the browser takes it to the device's.
                let options = web_sys::AudioContextOptions::new();
                options.set_sample_rate(48_000.0);
                i.ctx = AudioContext::new_with_context_options(&options)
                    .or_else(|_| AudioContext::new())
                    .ok();
            }
            if let Some(ctx) = &i.ctx {
                let _ = ctx.resume();
                if ctx.state() != web_sys::AudioContextState::Running && i.asked.is_none() {
                    i.asked = Some(architect_playhead::now_ms());
                }
            }
            i.say("run", &JsValue::TRUE);
            i.feed();
            let start = i.node.is_none() && !i.starting && i.ctx.is_some();
            i.starting |= start;
            start
        };
        if start {
            let this = self.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let made = this.worklet().await;
                let mut i = this.inner.borrow_mut();
                i.starting = false;
                match made {
                    Ok(node) => {
                        i.node = Some(node);
                        let at = i.time();
                        i.restart(at);
                        i.feed();
                    }
                    Err(why) => i.fail(&why),
                }
            });
        }
    }

    /// The worklet node, connected: its module from a blob of this page.
    async fn worklet(&self) -> Result<AudioWorkletNode, String> {
        let ctx = self.inner.borrow().ctx.clone().ok_or("no audio context")?;
        let parts = js_sys::Array::of1(&PROCESSOR.into());
        let kind = web_sys::BlobPropertyBag::new();
        kind.set_type("text/javascript");
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &kind).map_err(js)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
        let added = ctx
            .audio_worklet()
            .map_err(js)?
            .add_module(&url)
            .map_err(js)?;
        let loaded = JsFuture::from(added).await;
        let _ = web_sys::Url::revoke_object_url(&url);
        loaded.map_err(js)?;
        let options = web_sys::AudioWorkletNodeOptions::new();
        options.set_number_of_inputs(0);
        options.set_number_of_outputs(1);
        options.set_output_channel_count(&js_sys::Array::of1(&JsValue::from(2)));
        let node =
            AudioWorkletNode::new_with_options(&ctx, "architect-deck", &options).map_err(js)?;
        node.connect_with_audio_node(&ctx.destination())
            .map_err(js)?;
        // Its reports: what was played, and the cue to decode more.
        let this = self.clone();
        let on_report =
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
                let data = e.data();
                let number = |k: &str| {
                    js_sys::Reflect::get(&data, &k.into())
                        .ok()
                        .and_then(|v| v.as_f64())
                };
                let ended = {
                    let Ok(mut i) = this.inner.try_borrow_mut() else {
                        return;
                    };
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    if number("epoch").map(|e| e as u32) != Some(i.epoch) {
                        return;
                    }
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let played = number("played").unwrap_or(0.0) as u64;
                    let was = i.played;
                    i.played = played;
                    i.feed();
                    i.drained && i.posted > 0 && played >= i.posted && was < i.posted
                };
                // The end, heard at once (the deck goes to the next song).
                if ended {
                    (this.heard)();
                }
            });
        node.port()
            .map_err(js)?
            .set_onmessage(Some(on_report.as_ref().unchecked_ref()));
        on_report.forget();
        Ok(node)
    }

    /// Stop where it is: what is queued stays queued.
    pub fn pause(&self) {
        let mut i = self.inner.borrow_mut();
        i.want = false;
        i.say("run", &JsValue::FALSE);
    }

    /// To `t` seconds.
    pub fn seek(&self, t: f64) {
        let mut i = self.inner.borrow_mut();
        if i.stream.is_none() {
            i.pending = Some(t);
            i.base = t.max(0.0);
            return;
        }
        i.restart(t);
        i.feed();
    }

    /// It could not be decoded here: the element should play it.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.inner.borrow().failed
    }

    /// What it reads now, and more decoded if the queue ran low.
    #[must_use]
    pub fn read(&self) -> Reading {
        let Ok(mut i) = self.inner.try_borrow_mut() else {
            return Reading::default();
        };
        i.feed();
        // A context that never starts (no device, a browser that will not
        // run one here) is not a player: the element takes the recording.
        if let (Some(asked), Some(ctx)) = (i.asked, i.ctx.as_ref()) {
            if ctx.state() == web_sys::AudioContextState::Running {
                i.asked = None;
            } else if i.want && architect_playhead::now_ms() - asked > START_MS {
                i.asked = None;
                i.fail("the audio context did not start");
            }
        }
        let duration = i.duration();
        let ended = i.stream.is_some() && i.drained && i.posted > 0 && i.played >= i.posted;
        Reading {
            time: duration.map_or_else(|| i.time(), |d| i.time().min(d)),
            duration,
            moving: i.want && i.node.is_some() && i.posted > i.played,
            ended,
            failed: i.failed,
            resolved: true,
        }
    }
}

fn js(e: JsValue) -> String {
    e.as_string()
        .or_else(|| {
            js_sys::Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| "refused".to_owned())
}

/// The recording's bytes, from this origin, with no credentials (a link's
/// files are the link's; the node checks each request).
async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
    let response: web_sys::Response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "no response")?;
    if !response.ok() {
        return Err(format!("status {}", response.status()));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(js)?)
        .await
        .map_err(js)?;
    let bytes = js_sys::Uint8Array::new(&buffer);
    if bytes.length() > MAX_BYTES {
        return Err("too large to decode in the page".to_owned());
    }
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ogg_by_its_name_is_decoded_here() {
        assert!(handles("/s/abc/Songs/Joy/renders/joy.ogg"));
        assert!(handles("/s/abc/a.OGA?x=1"));
        assert!(!handles("/s/abc/a.mp3"));
        assert!(!handles("/s/abc/ogg/a.m4a"));
    }
}
