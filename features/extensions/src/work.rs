//! A process extension's heavy work, off its views' threads (feature
//! `guest`; `docs/content/architecture/extensions.md`, "Responsive by
//! design: views and workers").
//!
//! Each view of a process renders on a thread of its own, one event at a
//! time: a handler that computes for a second freezes its pane for that
//! second (the host marks it busy). The rule: a handler starts work and
//! returns; the work runs with [`spawn_work`] on a thread of its own,
//! reports its progress ([`Work::progress`]) and posts what it finds
//! ([`Work::post`]), and the view redraws from its [`WorkState`] as news
//! comes ([`use_work`]). Dropping the [`WorkHandle`] — the view closed,
//! the process told to stop — cancels it: the work sees
//! [`Work::cancelled`] and ends.
//!
//! ```ignore
//! let primes = use_work();
//! rsx! {
//!     button { onclick: move |_| primes.start(|work| count_primes(work, 2_000_000)), "Find primes" }
//!     progress { value: primes.state().fraction() }
//! }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value as Json;
use tokio::sync::watch;

/// Where a piece of work is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkStatus {
    /// Not started ([`use_work`] before [`UseWork::start`]).
    #[default]
    Idle,
    Running,
    Done,
    Failed,
    /// Cancelled before it finished.
    Cancelled,
}

/// How far a piece of work said it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub done: f64,
    pub total: f64,
    pub note: String,
}

impl Progress {
    /// `done / total`, in 0..=1 (0 with no total).
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.total > 0.0 {
            (self.done / self.total).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// A piece of work as its owner sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkState {
    pub status: WorkStatus,
    /// What it last reported.
    pub progress: Option<Progress>,
    /// What it posted, in order.
    pub messages: Vec<Json>,
    /// What it returned, or why it failed; `None` while it runs.
    pub result: Option<Result<Json, String>>,
}

impl WorkState {
    /// Its progress's fraction, 1 once done, 0 before any.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.status == WorkStatus::Done {
            return 1.0;
        }
        self.progress.as_ref().map_or(0.0, Progress::fraction)
    }
}

#[derive(Debug)]
struct Shared {
    state: Mutex<WorkState>,
    cancel: AtomicBool,
    /// Moves on every change: what an owner waits on.
    news: watch::Sender<u64>,
}

impl Shared {
    fn change(&self, f: impl FnOnce(&mut WorkState)) {
        f(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner));
        self.news.send_modify(|n| *n = n.wrapping_add(1));
    }

    fn snapshot(&self) -> WorkState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The work's side: report, post, and look whether to stop.
#[derive(Debug)]
pub struct Work {
    shared: Arc<Shared>,
}

impl Work {
    /// How far it is: `done` of `total`, with a note (the latest wins;
    /// call it as often as you like).
    pub fn progress(&self, done: f64, total: f64, note: impl Into<String>) {
        let note = note.into();
        self.shared.change(|s| {
            s.progress = Some(Progress { done, total, note });
        });
    }

    /// Something for its owner (a partial result).
    pub fn post(&self, message: Json) {
        self.shared.change(|s| s.messages.push(message));
    }

    /// Whether its owner went or cancelled it: stop soon.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::Relaxed)
    }

    /// `Err` once it is cancelled: `work.check()?` in a loop.
    ///
    /// # Errors
    ///
    /// It was cancelled.
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err("cancelled".to_owned())
        } else {
            Ok(())
        }
    }
}

/// The owner's side of a piece of work. Dropping it cancels the work.
#[derive(Debug)]
pub struct WorkHandle {
    shared: Arc<Shared>,
}

impl Drop for WorkHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl WorkHandle {
    /// Asks it to stop: [`Work::cancelled`] says so from now.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// Where it is now.
    #[must_use]
    pub fn state(&self) -> WorkState {
        self.shared.snapshot()
    }

    /// Something to wait on for its news: resolves `changed()` after
    /// each (they coalesce).
    #[must_use]
    pub fn news(&self) -> watch::Receiver<u64> {
        self.shared.news.subscribe()
    }
}

/// Runs `work` on a thread of its own, now: what it returns is its
/// result (`Err` its failure, or what [`Work::check`] gave once
/// cancelled). Its owner reads its [`WorkState`] from the handle.
pub fn spawn_work<F>(work: F) -> WorkHandle
where
    F: FnOnce(&Work) -> Result<Json, String> + Send + 'static,
{
    let shared = Arc::new(Shared {
        state: Mutex::new(WorkState {
            status: WorkStatus::Running,
            ..WorkState::default()
        }),
        cancel: AtomicBool::new(false),
        news: watch::Sender::new(0),
    });
    let theirs = Work {
        shared: Arc::clone(&shared),
    };
    let spawned = std::thread::Builder::new()
        .name("extension work".to_owned())
        .spawn(move || {
            let span = tracing::info_span!("ext.work");
            let _entered = span.enter();
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&theirs)))
                .unwrap_or_else(|_| Err("it panicked".to_owned()));
            let cancelled = theirs.cancelled();
            theirs.shared.change(|s| {
                s.status = match (&out, cancelled) {
                    (Ok(_), _) => WorkStatus::Done,
                    (Err(_), true) => WorkStatus::Cancelled,
                    (Err(_), false) => WorkStatus::Failed,
                };
                s.result = Some(out);
            });
        });
    if let Err(e) = spawned {
        shared.change(|s| {
            s.status = WorkStatus::Failed;
            s.result = Some(Err(format!("its thread did not start: {e}")));
        });
    }
    WorkHandle { shared }
}

/// A piece of work in a view: start it from a handler, read its state
/// as it goes. Copy; reading [`Self::state`] subscribes the component.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct UseWork {
    state: dioxus::prelude::Signal<WorkState>,
    handle: dioxus::prelude::Signal<Option<WorkHandle>>,
}

/// One piece of work for this component, cancelled when it unmounts
/// (its view closed) or when another is started.
#[must_use]
pub fn use_work() -> UseWork {
    use dioxus::prelude::use_signal;
    UseWork {
        state: use_signal(WorkState::default),
        handle: use_signal(|| None),
    }
}

impl UseWork {
    /// Starts `work` on a thread of its own, cancelling one running.
    pub fn start<F>(&self, work: F)
    where
        F: FnOnce(&Work) -> Result<Json, String> + Send + 'static,
    {
        use dioxus::prelude::{WritableExt as _, spawn};
        let handle = spawn_work(work);
        let mut news = handle.news();
        let shared = Arc::clone(&handle.shared);
        let mut state = self.state;
        state.set(shared.snapshot());
        // The old one, if any, is dropped (cancelled) here.
        let mut slot = self.handle;
        slot.set(Some(handle));
        spawn(async move {
            loop {
                if news.changed().await.is_err() {
                    return;
                }
                let now = shared.snapshot();
                let over = !matches!(now.status, WorkStatus::Running);
                state.set(now);
                if over {
                    return;
                }
            }
        });
    }

    /// Cancels it.
    pub fn cancel(&self) {
        use dioxus::prelude::ReadableExt as _;
        if let Some(h) = self.handle.peek().as_ref() {
            h.cancel();
        }
    }

    /// Where it is. Reactive.
    #[must_use]
    pub fn state(&self) -> WorkState {
        use dioxus::prelude::ReadableExt as _;
        self.state.read().clone()
    }

    /// Whether it is running. Reactive.
    #[must_use]
    pub fn running(&self) -> bool {
        use dioxus::prelude::ReadableExt as _;
        self.state.read().status == WorkStatus::Running
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use serde_json::json;

    use super::{WorkStatus, spawn_work};

    #[test]
    fn work_reports_progress_and_its_result_off_the_owners_thread() {
        let owner = std::thread::current().id();
        let handle = spawn_work(move |work| {
            assert_ne!(std::thread::current().id(), owner);
            for i in 1..=50 {
                work.check()?;
                std::thread::sleep(Duration::from_millis(1));
                work.progress(f64::from(i), 50.0, format!("{i} of 50"));
                if i == 25 {
                    work.post(json!({ "half": true }));
                }
            }
            Ok(json!(1229))
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let seen = rt.block_on(async {
            let mut news = handle.news();
            let mut seen = Vec::new();
            loop {
                let s = handle.state();
                seen.push(s.fraction());
                if s.status != WorkStatus::Running {
                    break;
                }
                if news.changed().await.is_err() {
                    break;
                }
            }
            seen
        });
        let end = handle.state();
        assert_eq!(end.status, WorkStatus::Done);
        assert_eq!(end.result, Some(Ok(json!(1229))));
        assert_eq!(end.messages, [json!({ "half": true })]);
        assert!(seen.len() > 3, "news as it went: {seen:?}");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert!((end.fraction() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn dropping_the_handle_cancels_the_work() {
        let (ended, end) = mpsc::channel();
        let handle = spawn_work(move |work| {
            while !work.cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            let _ = ended.send(());
            work.check().map(|()| json!(null))
        });
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(handle.state().status, WorkStatus::Running);
        drop(handle);
        assert!(end.recv_timeout(Duration::from_secs(2)).is_ok(), "it ended");
    }
}
