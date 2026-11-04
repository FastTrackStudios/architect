//! Where Luau runs: never on its caller's thread (feature `luau`).
//!
//! A one-off run (a script, an action, a hook) and a worker each take a
//! thread of a small pool that grows when every thread is busy and lets
//! one go after it has idled a while ([`spawn`]); a live script (a view,
//! an `ImGui` window) has a thread of its own for its life
//! ([`crate::luau::Live`]). Each Luau state lives on the one thread that
//! made it; what crosses is JSON and draw lists. The caller awaits a
//! channel, so an app's UI thread never waits on a script.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use futures::channel::oneshot;

type Job = Box<dyn FnOnce() + Send>;

/// How long a pool thread waits for work before it ends.
const IDLE_FOR: Duration = Duration::from_secs(30);

struct Pool {
    state: Mutex<PoolState>,
    ready: Condvar,
}

#[derive(Default)]
struct PoolState {
    jobs: VecDeque<Job>,
    idle: usize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        state: Mutex::new(PoolState::default()),
        ready: Condvar::new(),
    })
}

/// Runs `job` on a pool thread: an idle one, or a new one when none is
/// (a run that waits on another — a script running a script — never
/// waits for a thread).
pub fn spawn(job: impl FnOnce() + Send + 'static) {
    let pool = pool();
    let mut state = lock(&pool.state);
    state.jobs.push_back(Box::new(job));
    if state.jobs.len() <= state.idle {
        pool.ready.notify_one();
        return;
    }
    drop(state);
    let started = std::thread::Builder::new()
        .name("luau vm".to_owned())
        .spawn(serve);
    if let Err(e) = started {
        // No thread to be had: the job waits for one that frees up.
        tracing::warn!(error = %e, "a Luau thread did not start");
        pool.ready.notify_one();
    }
}

fn serve() {
    let pool = pool();
    loop {
        let mut state = lock(&pool.state);
        let job = loop {
            if let Some(job) = state.jobs.pop_front() {
                break job;
            }
            state.idle = state.idle.saturating_add(1);
            let (next, waited) = pool
                .ready
                .wait_timeout(state, IDLE_FOR)
                .unwrap_or_else(PoisonError::into_inner);
            state = next;
            state.idle = state.idle.saturating_sub(1);
            if waited.timed_out() && state.jobs.is_empty() {
                return;
            }
        };
        drop(state);
        job();
    }
}

/// A flag that stops a run: set, its next step raises.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// A run on the pool: resolves to what it made; dropped, it cancels it.
pub struct Task<T> {
    rx: oneshot::Receiver<T>,
    cancel: Cancel,
}

impl<T> Future for Task<T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        Pin::new(&mut self.rx).poll(cx).map(Result::ok)
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// Runs the future `make` makes on a pool thread, to its end: `make` is
/// called there (its Luau state is made on the thread it lives on), with
/// the [`Cancel`] dropping the [`Task`] sets.
pub fn run<T, F, Fut>(make: F) -> Task<T>
where
    T: Send + 'static,
    F: FnOnce(Cancel) -> Fut + Send + 'static,
    Fut: Future<Output = T>,
{
    let (tx, rx) = oneshot::channel();
    let cancel = Cancel::default();
    let theirs = cancel.clone();
    spawn(move || {
        let out = futures::executor::block_on(make(theirs));
        let _ = tx.send(out);
    });
    Task { rx, cancel }
}

/// A process-wide origin for deadlines kept as atomics.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn nanos_since_epoch(at: Instant) -> u64 {
    u64::try_from(at.saturating_duration_since(epoch()).as_nanos()).unwrap_or(u64::MAX)
}

/// Why a [`Guard`] stopped a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Its step limit.
    Steps(u64),
    /// Its time: a frame's budget, a worker's limit.
    Time(Duration),
    /// It was cancelled: its owner went, or it was told to stop.
    Cancelled,
}

/// What a state's interrupt checks at each step: its steps, a deadline
/// and its cancel flags.
#[derive(Debug)]
pub struct Guard {
    steps: AtomicU64,
    limit: u64,
    /// Nanoseconds since [`epoch`]; zero for none.
    deadline: AtomicU64,
    allowed: Mutex<Duration>,
    cancel: Vec<Cancel>,
}

impl Guard {
    pub const fn new(limit: u64, cancel: Vec<Cancel>) -> Self {
        Self {
            steps: AtomicU64::new(0),
            limit,
            deadline: AtomicU64::new(0),
            allowed: Mutex::new(Duration::ZERO),
            cancel,
        }
    }

    /// Steps start again from none.
    pub fn reset(&self) {
        self.steps.store(0, Ordering::Relaxed);
    }

    /// From now, the run has `allowed` (none: as long as it likes).
    pub fn deadline(&self, allowed: Option<Duration>) {
        let at = allowed.map_or(0, |d| {
            nanos_since_epoch(Instant::now().checked_add(d).unwrap_or_else(Instant::now)).max(1)
        });
        *lock(&self.allowed) = allowed.unwrap_or_default();
        self.deadline.store(at, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.iter().any(Cancel::is_set)
    }

    /// One step: why the run must stop, when it must.
    pub fn step(&self) -> Option<Stop> {
        let n = self.steps.fetch_add(1, Ordering::Relaxed);
        if n >= self.limit {
            return Some(Stop::Steps(self.limit));
        }
        // The clock and the flags every so many steps: a step is cheap.
        if !n.is_multiple_of(256) {
            return None;
        }
        if self.cancelled() {
            return Some(Stop::Cancelled);
        }
        let deadline = self.deadline.load(Ordering::Relaxed);
        if deadline != 0 && nanos_since_epoch(Instant::now()) > deadline {
            return Some(Stop::Time(*lock(&self.allowed)));
        }
        None
    }
}

/// Wakes a live script's host for a frame with no input: a worker's news
/// arrived. Cheap to clone; wakes coalesce.
#[derive(Clone, Debug, Default)]
pub struct Wake(Arc<WakeInner>);

#[derive(Debug, Default)]
struct WakeInner {
    due: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl Wake {
    /// Asks for a frame.
    pub fn wake(&self) {
        self.0.due.store(true, Ordering::Release);
        let waker = lock(&self.0.waker).take();
        if let Some(w) = waker {
            w.wake();
        }
    }

    /// Resolves once a frame is asked for (at once if one was since the
    /// last), taking the ask.
    pub fn woken(&self) -> impl Future<Output = ()> + '_ {
        std::future::poll_fn(move |cx| {
            if self.0.due.swap(false, Ordering::AcqRel) {
                return Poll::Ready(());
            }
            *lock(&self.0.waker) = Some(cx.waker().clone());
            // Asked between the check and the waker going in.
            if self.0.due.swap(false, Ordering::AcqRel) {
                return Poll::Ready(());
            }
            Poll::Pending
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::{Cancel, Guard, Stop, Wake, run};

    #[test]
    fn runs_that_wait_on_each_other_each_get_a_thread() {
        // Eight runs, each waiting for the next: a fixed pool smaller
        // than that would deadlock.
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<usize>();
        let done = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        for n in 0..8 {
            let (tx, done, barrier) = (tx.clone(), Arc::clone(&done), Arc::clone(&barrier));
            tasks.push(run(move |_| async move {
                barrier.wait();
                done.fetch_add(1, Ordering::Relaxed);
                let _ = tx.unbounded_send(n);
                n
            }));
        }
        let got: Vec<Option<usize>> = tasks.into_iter().map(futures::executor::block_on).collect();
        assert_eq!(got, (0..8).map(Some).collect::<Vec<_>>());
        assert_eq!(done.load(Ordering::Relaxed), 8);
        drop(tx);
        assert_eq!(futures::executor::block_on_stream(&mut rx).count(), 8);
    }

    #[test]
    fn a_guard_stops_at_its_steps_its_deadline_and_its_cancel() {
        let g = Guard::new(1000, Vec::new());
        assert_eq!((0..1001).find_map(|_| g.step()), Some(Stop::Steps(1000)));
        let cancel = Cancel::default();
        let g = Guard::new(u64::MAX, vec![cancel.clone()]);
        assert_eq!(g.step(), None);
        cancel.set();
        assert_eq!((0..300).find_map(|_| g.step()), Some(Stop::Cancelled));
        let g = Guard::new(u64::MAX, Vec::new());
        g.deadline(Some(Duration::from_millis(1)));
        std::thread::sleep(Duration::from_millis(3));
        assert_eq!(
            (0..300).find_map(|_| g.step()),
            Some(Stop::Time(Duration::from_millis(1)))
        );
    }

    #[test]
    fn wakes_coalesce() {
        let w = Wake::default();
        w.wake();
        w.wake();
        futures::executor::block_on(w.woken());
        let w2 = w.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            w2.wake();
        });
        futures::executor::block_on(w.woken());
        t.join().ok();
    }
}
