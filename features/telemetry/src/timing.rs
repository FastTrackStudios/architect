//! Where the time goes: stage timings on the current unit of work, kept
//! in-process to read back.
//!
//! A unit of work is a span — an Action List action, an RPC, a frame's
//! edit. [`Stages`] marks how long each named stage of it took; the
//! marks are structured `tracing` events, so they ride whatever the
//! subscriber already does (OTLP export, the fmt log). The [`Recorder`]
//! layer folds them into their span and, when the span closes, keeps the
//! whole thing — name, fields, stages, total — in a ring buffer the app
//! can read back ([`recent`]): a model asks the running app where the
//! last edit's time went, and a benchmark asserts on it, with no prints
//! and no OTel collector.
//!
//! Stage keys follow the wide-event conventions: dotted, namespaced,
//! static, the unit in the name — `edit.detect_ms`, `edit.slip_ms`.
//!
//! ```ignore
//! let mut stages = architect_telemetry::timing::Stages::start();
//! let found = detect(..);
//! stages.mark("edit.detect_ms");
//! slip(found);
//! stages.mark("edit.slip_ms");
//! ```

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// The `tracing` target stage marks are sent on.
pub const TARGET: &str = "architect::timing";

/// The directive that lets stage marks through a global `EnvFilter`
/// (they are trace-level, under every default):
/// `filter.add_directive(timing::DIRECTIVE.parse()?)`.
pub const DIRECTIVE: &str = "architect::timing=trace";

/// Every directive the recorder needs through a global `EnvFilter`.
///
/// Whatever the log level, it needs the stage marks and the Action
/// List's span. Without the span, under `RUST_LOG=warn`, no action is
/// timed and the marks made in one have no span to land on.
pub const DIRECTIVES: [&str; 2] = [DIRECTIVE, "architect::action_list=info"];

/// A per-layer filter that keeps stage marks out of a layer that would
/// print them (a console log, a log ring): what they say is the
/// [`Recorder`]'s to keep, once per span, not a line per mark.
#[must_use]
pub fn no_marks<S>() -> impl tracing_subscriber::layer::Filter<S> {
    tracing_subscriber::filter::filter_fn(|meta| meta.target() != TARGET)
}

/// The stages of one unit of work, each timed from the mark before it.
#[derive(Debug)]
pub struct Stages {
    started: Instant,
    last: Instant,
}

impl Stages {
    /// Start timing now.
    #[must_use]
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
        }
    }

    /// The time since the last mark (or the start), recorded on the
    /// current span as `key` (`edit.detect_ms`). Returns it, ms.
    pub fn mark(&mut self, key: &'static str) -> f64 {
        let now = Instant::now();
        let ms = now.duration_since(self.last).as_secs_f64() * 1e3;
        self.last = now;
        record(key, ms);
        ms
    }

    /// The time since the start, ms.
    #[must_use]
    pub fn total_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1e3
    }
}

/// Record `ms` as `key` on the current span — a duration measured some
/// other way (another thread's, a callback's).
pub fn record(key: &'static str, ms: f64) {
    tracing::trace!(target: TARGET, stage = key, ms);
}

/// One unit of work, as the [`Recorder`] kept it.
#[derive(Clone, Debug, PartialEq)]
pub struct Timed {
    /// The span's name (`action`, `window.slip`).
    pub name: &'static str,
    /// Its fields, as recorded (`action.id = QUANTIZE`).
    pub fields: Vec<(&'static str, String)>,
    /// Its stages in the order marked, ms.
    pub stages: Vec<(&'static str, f64)>,
    /// From opening to closing, ms.
    pub total_ms: f64,
    /// When it closed.
    pub at: SystemTime,
}

impl Timed {
    /// A field's value, if it has it.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    /// A stage's time, ms (summed when marked more than once).
    #[must_use]
    pub fn stage(&self, key: &str) -> Option<f64> {
        let mut found = None;
        for (k, ms) in &self.stages {
            if *k == key {
                *found.get_or_insert(0.0) += ms;
            }
        }
        found
    }
}

/// How many units of work are kept.
const KEPT: usize = 512;

static RECENT: Mutex<VecDeque<Timed>> = Mutex::new(VecDeque::new());

/// The latest `n` units of work kept, newest first.
#[must_use]
pub fn recent(n: usize) -> Vec<Timed> {
    RECENT
        .lock()
        .map(|kept| kept.iter().rev().take(n).cloned().collect())
        .unwrap_or_default()
}

/// Forget what was kept — a benchmark's clean slate.
pub fn clear() {
    if let Ok(mut kept) = RECENT.lock() {
        kept.clear();
    }
}

fn keep(timed: Timed) {
    if let Ok(mut kept) = RECENT.lock() {
        if kept.len() >= KEPT {
            kept.pop_front();
        }
        kept.push_back(timed);
    }
}

/// A span being timed: what [`Timed`] is made from when it closes.
struct Open {
    opened: Instant,
    fields: Vec<(&'static str, String)>,
    stages: Vec<(&'static str, f64)>,
}

/// The layer that keeps timed units of work ([`recent`]): every span
/// that had a stage marked in it, and every span named in `always` (an
/// action, whether it marked stages or not).
#[derive(Clone, Debug)]
pub struct Recorder {
    always: &'static [&'static str],
}

impl Recorder {
    /// Keep the spans with stages, and every span named one of `always`.
    #[must_use]
    pub const fn new(always: &'static [&'static str]) -> Self {
        Self { always }
    }
}

impl Default for Recorder {
    /// Spans with stages, and every Action List action.
    fn default() -> Self {
        Self::new(&["action"])
    }
}

struct Fields<'a>(&'a mut Vec<(&'static str, String)>);

impl Visit for Fields<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        self.0.push((field.name(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push((field.name(), value.to_owned()));
    }
}

/// A stage mark's key and time.
#[derive(Default)]
struct Mark {
    stage: Option<String>,
    ms: Option<f64>,
}

impl Visit for Mark {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        if field.name() == "stage" {
            self.stage = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "stage" {
            self.stage = Some(value.to_owned());
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "ms" {
            self.ms = Some(value);
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for Recorder
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut fields = Vec::new();
        attrs.record(&mut Fields(&mut fields));
        span.extensions_mut().insert(Open {
            opened: Instant::now(),
            fields,
            stages: Vec::new(),
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut ext = span.extensions_mut();
        if let Some(open) = ext.get_mut::<Open>() {
            let mut fields = Vec::new();
            values.record(&mut Fields(&mut fields));
            for (k, v) in fields {
                open.fields.retain(|(have, _)| *have != k);
                open.fields.push((k, v));
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if event.metadata().target() != TARGET {
            return;
        }
        let mut mark = Mark::default();
        event.record(&mut mark);
        let (Some(stage), Some(ms)) = (mark.stage, mark.ms) else {
            return;
        };
        let Some(span) = ctx.event_span(event) else {
            return;
        };
        let mut ext = span.extensions_mut();
        if let Some(open) = ext.get_mut::<Open>() {
            // Interned once per key: stage keys are a small, fixed set.
            open.stages.push((intern(&stage), ms));
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let name = span.name();
        let Some(open) = span.extensions_mut().remove::<Open>() else {
            return;
        };
        if open.stages.is_empty() && !self.always.contains(&name) {
            return;
        }
        keep(Timed {
            name,
            fields: open.fields,
            stages: open.stages,
            total_ms: open.opened.elapsed().as_secs_f64() * 1e3,
            at: SystemTime::now(),
        });
    }
}

/// `key` as a `&'static str`: each distinct key leaked once.
fn intern(key: &str) -> &'static str {
    static KEYS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let Ok(mut keys) = KEYS.lock() else {
        return "?";
    };
    if let Some(k) = keys.iter().find(|k| **k == key) {
        return k;
    }
    let leaked: &'static str = Box::leak(key.to_owned().into_boxed_str());
    keys.push(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt as _;

    #[test]
    fn a_span_keeps_its_stages_and_fields() {
        let subscriber = tracing_subscriber::registry().with(Recorder::default());
        tracing::subscriber::with_default(subscriber, || {
            let span =
                tracing::info_span!("work", work.kind = "slip", work.ok = tracing::field::Empty);
            {
                let _in = span.enter();
                let mut stages = Stages::start();
                stages.mark("work.read_ms");
                stages.mark("work.edit_ms");
                record("work.read_ms", 2.0);
                span.record("work.ok", true);
            }
            drop(span);
            // A span with no stages and not named `always` is not kept.
            drop(tracing::info_span!("noise"));
            // Tests share the buffer: this test's span, by its field.
            let kept = recent(KEPT);
            assert!(kept.iter().all(|t| t.name != "noise"), "{kept:?}");
            let work = kept
                .iter()
                .find(|t| t.field("work.kind") == Some("slip"))
                .expect("the span kept");
            assert_eq!(work.name, "work");
            assert_eq!(work.field("work.kind"), Some("slip"));
            assert_eq!(work.field("work.ok"), Some("true"));
            assert_eq!(work.stages.len(), 3);
            assert!(work.stage("work.read_ms").is_some_and(|ms| ms >= 2.0));
            assert!(work.stage("work.edit_ms").is_some());
        });
    }

    #[test]
    fn an_action_is_kept_without_stages() {
        let subscriber = tracing_subscriber::registry().with(Recorder::default());
        tracing::subscriber::with_default(subscriber, || {
            drop(tracing::info_span!("action", action.id = "QUANTIZE"));
            assert!(
                recent(KEPT)
                    .iter()
                    .any(|t| t.name == "action" && t.field("action.id") == Some("QUANTIZE"))
            );
        });
    }
}
