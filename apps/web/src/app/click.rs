//! Click (#589): a tick on each beat of the active item's `tempo`, while it
//! plays and the bar's Click is on (`architect_playhead::click`). The
//! share page's player's click (`features/sites/assets/player.js`), here
//! for the app: scheduled a little ahead on the Web Audio clock from where
//! the deck says the item is, never a timer a beat; a jump (a seek,
//! another item, a rate change) silences what was scheduled and starts
//! again from there.
//!
//! The audio context is made and woken inside a press (a browser starts
//! one only there): any press while Click is on, the Click press itself
//! included.

use std::cell::RefCell;

use architect_playhead::{Tempo, now_ms};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use web_sys::{AudioContext, AudioContextState, OscillatorNode};

/// How far ahead beats are scheduled, seconds; how often it looks, ms;
/// how far the item may be from where it was expected before it is a jump.
const AHEAD: f64 = 0.15;
const EVERY_MS: u64 = 25;
const JUMP: f64 = 0.25;

#[derive(Default)]
struct Metro {
    ctx: Option<AudioContext>,
    /// What it follows: the source, its item, and the item's tempo.
    key: Option<String>,
    /// Song time beats are scheduled up to.
    upto: Option<f64>,
    /// Where the item was and when (song seconds, context seconds, rate).
    at: Option<(f64, f64, f64)>,
    voices: Vec<OscillatorNode>,
}

impl Metro {
    fn hush(&mut self) {
        for v in self.voices.drain(..) {
            #[allow(deprecated)]
            let _ = v.stop_with_when(0.0);
        }
        self.upto = None;
        self.at = None;
    }

    fn voice(&mut self, when: f64, accent: bool) {
        let Some(c) = &self.ctx else { return };
        let (Ok(o), Ok(g)) = (c.create_oscillator(), c.create_gain()) else {
            return;
        };
        o.frequency()
            .set_value(if accent { 1760.0 } else { 1175.0 });
        let gain = g.gain();
        let _ = gain.set_value_at_time(0.0001, when);
        let _ =
            gain.exponential_ramp_to_value_at_time(if accent { 0.5 } else { 0.3 }, when + 0.002);
        let _ = gain.exponential_ramp_to_value_at_time(0.0001, when + 0.045);
        let _ = o.connect_with_audio_node(&g);
        let _ = g.connect_with_audio_node(&c.destination());
        let _ = o.start_with_when(when);
        let _ = o.stop_with_when(when + 0.05);
        // Kept to be silenced on a jump: only the last few are still ahead.
        if self.voices.len() >= 16 {
            self.voices.drain(..8);
        }
        self.voices.push(o);
    }
}

thread_local! {
    static METRO: RefCell<Metro> = RefCell::new(Metro::default());
}

/// Inside a press: the context made, and woken, while Click is on.
fn wake() {
    if !architect_playhead::click() {
        return;
    }
    METRO.with(|m| {
        let mut m = m.borrow_mut();
        if m.ctx.is_none() {
            m.ctx = AudioContext::new().ok();
        }
        if let Some(c) = &m.ctx
            && c.state() == AudioContextState::Suspended
        {
            let _ = c.resume();
        }
    });
}

/// What the active source plays now: its key, its time, its rate, whether
/// it plays, its item's tempo.
fn playing_now() -> Option<(String, f64, f64, Tempo)> {
    let now = now_ms();
    architect_playhead::with(|d| {
        let s = d.current()?;
        let item = s.item()?;
        let tempo = item.tempo?;
        s.transport.playing.then(|| {
            (
                format!(
                    "{}|{}|{}|{}|{}",
                    s.id, item.id, tempo.bpm, tempo.at, s.epoch
                ),
                s.time(now),
                s.transport.rate,
                tempo,
            )
        })
    })
}

fn tick() {
    let on = architect_playhead::click();
    let now = playing_now();
    METRO.with(|m| {
        let mut m = m.borrow_mut();
        let running = m
            .ctx
            .as_ref()
            .is_some_and(|c| c.state() == AudioContextState::Running);
        let (Some((key, time, rate, tempo)), true, true) = (now, on, running) else {
            if !m.voices.is_empty() || m.at.is_some() {
                m.hush();
            }
            m.key = None;
            return;
        };
        let Some(ctx_now) = m.ctx.as_ref().map(AudioContext::current_time) else {
            return;
        };
        let rate = if rate > 0.0 { rate } else { 1.0 };
        if m.key.as_deref() != Some(key.as_str()) {
            m.hush();
            m.key = Some(key);
        }
        if let Some((t0, c0, r0)) = m.at {
            let expect = t0 + (ctx_now - c0) * r0;
            if (time - expect).abs() > JUMP || (rate - r0).abs() > f64::EPSILON {
                m.hush();
            }
        }
        // A beat just passed still sounds (30 ms late at most).
        let from = m.upto.unwrap_or(time - 0.03);
        let to = time + AHEAD * rate;
        for (at, accent) in tempo.beats(from, to) {
            let dt = (at - time) / rate;
            if dt >= -0.03 {
                m.voice(ctx_now + dt.max(0.0), accent);
            }
        }
        m.upto = Some(to.max(from));
        m.at = Some((time, ctx_now, rate));
    });
}

/// Says the app sounds Click and starts following the deck: once, at the
/// app's start.
pub fn start() {
    architect_playhead::sounds_click();
    let Some(w) = web_sys::window() else { return };
    // After the press's own handler (Click's toggle among them), so a
    // Click turned on is woken by the press that turned it on.
    for ev in ["click", "keydown", "pointerdown"] {
        let on = Closure::<dyn FnMut()>::new(wake);
        let _ = w.add_event_listener_with_callback(ev, on.as_ref().unchecked_ref());
        on.forget();
    }
    wasm_bindgen_futures::spawn_local(async {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(EVERY_MS)).await;
            tick();
        }
    });
}
