//! A value that eases toward a target a frame at a time, or is held where
//! a finger puts it: a drawer's travel. Driven by a timer, not CSS, so it
//! moves the same in Blitz as in a browser.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;

/// About a display frame.
const FRAME: Duration = Duration::from_millis(16);
/// The share of the distance left that each frame covers: an ease-out
/// that settles in about a fifth of a second.
const EASE: f64 = 0.3;
/// Close enough to call it there.
const SETTLED: f64 = 0.004;

#[derive(Clone, Copy)]
pub(super) struct Motion {
    value: Signal<f64>,
    /// Bumped by every new target or hold: a run that sees it change stops.
    run: CopyValue<Rc<Cell<u64>>>,
}

pub(super) fn use_motion() -> Motion {
    Motion {
        value: use_signal(|| 0.0),
        run: use_hook(|| CopyValue::new(Rc::new(Cell::new(0)))),
    }
}

impl Motion {
    pub(super) fn value(self) -> f64 {
        (self.value)()
    }

    fn next_run(self) -> (Rc<Cell<u64>>, u64) {
        let run = self.run.read().clone();
        let n = run.get().wrapping_add(1);
        run.set(n);
        (run, n)
    }

    /// Put it here and keep it here (a finger has it).
    pub(super) fn hold(self, at: f64) {
        let _ = self.next_run();
        let mut value = self.value;
        value.set(at);
    }

    /// Ease to `to`, then call `done`.
    pub(super) fn to(self, to: f64, mut done: impl FnMut() + 'static) {
        let (run, n) = self.next_run();
        let mut value = self.value;
        spawn(async move {
            loop {
                futures_timer::Delay::new(FRAME).await;
                if run.get() != n {
                    return;
                }
                let at = *value.peek();
                let next = (to - at).mul_add(EASE, at);
                if (to - next).abs() < SETTLED {
                    value.set(to);
                    done();
                    return;
                }
                value.set(next);
            }
        });
    }
}
