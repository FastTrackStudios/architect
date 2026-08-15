//! Rig-backend glue — the shared scaffold every instrument-rig backend
//! serves through, so rigs **cannot drift**.
//!
//! A rig backend (guitar / drums / keys / synth) is a cloneable handle over
//! shared state that (a) mounts its `#[architect::rpc]` services as a
//! [`LayerRouter`], (b) fans live updates out of a [`PubSub`] hub behind a
//! `#[subscribe]` stream, and (c) runs one background **meter/status pump**:
//! a fixed-interval loop that publishes per-tick telemetry (meters + recent
//! MIDI), publishes full status only when the transport running-state flips,
//! and periodically rescans MIDI ports for hot-plug. That plumbing was
//! copy-pasted per rig — and drifted (one rig missed the hot-plug rescan,
//! another the once-start guard, each declared its own interval constant).
//!
//! [`RigBackend`] is the one scaffold: `router()` is provided
//! (`self.clone().into_router()`), the hub comes from [`events_hub()`]
//! (`PubSub::sliding(EVENT_BACKLOG)`), and
//! [`spawn_meter_pump`](RigBackend::spawn_meter_pump) ships the interval, the
//! once-start guard, the running-edge detection, the hot-plug rescan, and
//! per-tick panic containment. A rig implements only what is
//! instrument-specific: its event enum, its service methods, and the pump
//! hooks' bodies.
//!
//! The pump is deliberately a plain OS thread, not an async task:
//! [`PubSub::publish`] takes a lock (never call it on a real-time thread) and
//! the loop is a 30 Hz sleeper with no await points — a dedicated named
//! thread is the honest shape, and it must keep running even if every async
//! runtime is busy.

// r[impl primitives.architect.rig-backend]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::layer::{LayerRouter, Services};
use crate::pubsub::PubSub;

/// Cadence of the meter/status pump loop (~30 Hz). The **one** interval
/// constant — rigs must not declare their own.
pub const PUMP_INTERVAL: Duration = Duration::from_millis(33);

/// Sliding-backlog depth of a rig's event hub (see [`events_hub`]).
pub const EVENT_BACKLOG: usize = 64;

/// Hot-plug rescan cadence, in pump ticks (~2 s at [`PUMP_INTERVAL`]).
const PORT_SCAN_TICKS: u32 = 60;

/// The canonical rig event hub: `PubSub::sliding(EVENT_BACKLOG)`. Sliding is
/// the right strategy for state-shaped rig events (a newer `Status`
/// supersedes older news; a slow subscriber must never block the publisher).
pub fn events_hub<E>() -> PubSub<E> {
    PubSub::sliding(EVENT_BACKLOG)
}

/// The shared rig-backend scaffold. Implement the instrument-specific hooks;
/// `router()` and the pump loop are provided and identical for every rig.
pub trait RigBackend: Services + Clone + Send + Sync + Sized + 'static {
    /// The rig's `#[subscribe]` event enum.
    type Event;

    /// Pump-thread-local scratch state, created on the pump thread and fed to
    /// [`on_tick`](Self::on_tick) each iteration (kept outside the tick so a
    /// caught panic in one iteration doesn't reset it). `()` when unused.
    type Tick: Default;

    /// The event hub behind the rig's `#[subscribe]` stream — construct it
    /// with [`events_hub()`].
    fn events_hub(&self) -> &PubSub<Self::Event>;

    /// The composed service router — mount this on a vox transport
    /// (in-process, WebSocket, iroh). Provided: the canonical bundle.
    fn router(&self) -> LayerRouter {
        self.clone().into_router()
    }

    // ── meter/status pump hooks ──────────────────────────────────────────

    /// The once-start guard: at most one pump thread per backend, however
    /// many times [`spawn_meter_pump`](Self::spawn_meter_pump) is called
    /// (construction, `start`, `load_*` — whichever comes first wins).
    fn pump_started(&self) -> &AtomicBool;

    /// True while the audio engine is open/running.
    fn is_running(&self) -> bool;

    /// Run once per tick, before the running check (e.g. decay a light
    /// guide, drain a footswitch stream, flush debounced saves). Default:
    /// nothing.
    fn on_tick(&self, _tick: &mut Self::Tick) {}

    /// Publish full state on a transport running-state edge (started /
    /// stopped) — edges are rare, full snapshots belong here.
    fn on_running_edge(&self, running: bool);

    /// Publish per-tick telemetry (meters, recent MIDI) while running.
    fn on_running_tick(&self);

    /// Sorted MIDI input port names, for hot-plug detection. Default: no
    /// hot-plug (an always-empty set never fires
    /// [`on_midi_ports_changed`](Self::on_midi_ports_changed)).
    fn midi_ports(&self) -> Vec<String> {
        Vec::new()
    }

    /// The MIDI port set changed *while running* — re-attach. Default:
    /// nothing.
    fn on_midi_ports_changed(&self, _ports: &[String]) {}

    /// Spawn the meter/status pump on a named thread. Idempotent (see
    /// [`pump_started`](Self::pump_started)). Provided — rigs never
    /// hand-roll the loop.
    fn spawn_meter_pump(&self, name: &'static str) {
        spawn_meter_pump(name, self.clone());
    }
}

/// The pump loop behind [`RigBackend::spawn_meter_pump`]. Each iteration runs
/// under `catch_unwind`: the pump is a rig's control heartbeat (telemetry,
/// hot-plug, and whatever [`on_tick`](RigBackend::on_tick) drives — gestures,
/// auto-saves), so it must survive the whole service. A panic — its own, or
/// one surfacing through a handler it calls — is logged and the loop keeps
/// going instead of dying silently while audio plays on.
pub fn spawn_meter_pump<S: RigBackend>(name: &'static str, source: S) {
    if source.pump_started().swap(true, Ordering::SeqCst) {
        return; // already running
    }
    let spawned = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let mut last_running = false;
            let mut known_ports = sorted(source.midi_ports());
            let mut tick_no: u32 = 0;
            let mut tick = S::Tick::default();
            loop {
                std::thread::sleep(PUMP_INTERVAL);
                let iteration = catch_unwind(AssertUnwindSafe(|| {
                    source.on_tick(&mut tick);
                    let running = source.is_running();

                    // Hot-plug: rescan periodically; on a change re-attach
                    // (only while running — a stopped rig has nothing to
                    // attach to).
                    tick_no = tick_no.wrapping_add(1);
                    if tick_no.is_multiple_of(PORT_SCAN_TICKS) {
                        let now = sorted(source.midi_ports());
                        // An EMPTY scan is a transient enumeration failure,
                        // not "every keyboard was unplugged": with several
                        // rigs in one process each opening a client to
                        // enumerate, JACK/ALSA refuses one now and then.
                        // Tearing down a live attachment on that reads as
                        // "MIDI just stopped working". A real unplug still
                        // resolves — the dead stream re-attaches on the next
                        // non-empty scan.
                        let transient = now.is_empty() && !known_ports.is_empty();
                        if !transient && now != known_ports {
                            known_ports = now;
                            if running {
                                source.on_midi_ports_changed(&known_ports);
                            }
                        }
                    }

                    // Transport transitions are rare — full status only on
                    // the edge.
                    if running != last_running {
                        last_running = running;
                        source.on_running_edge(running);
                    }
                    if running {
                        source.on_running_tick();
                    }
                }));
                if let Err(panic) = iteration {
                    tracing::error!(
                        "meter pump '{name}': tick panicked ({}) — pump stays alive",
                        panic_message(&*panic)
                    );
                }
            }
        });
    if let Err(e) = spawned {
        tracing::error!("meter pump '{name}': spawn failed: {e}");
    }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// Best-effort text from a caught panic payload.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}
