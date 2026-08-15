//! [`AtomResult`] — stale-while-revalidate async state.
//!
//! effect-atom models async data with a `Result` whose `Initial / Waiting
//! / Success / Failure` phases *retain the previous value across a
//! refetch*. Dioxus's `use_resource` doesn't: its `Option<Result<T, E>>`
//! drops to `None` on every restart, so the UI flashes empty mid-reload.
//! `AtomResult` is the same idea ported to a plain Rust enum — render a
//! page by `match`ing it directly (effect needs an object-matcher because
//! JS lacks pattern matching; Rust has `match`).
//!
//! The phases are flat variants, so a page can `use architect::AtomResult::*`
//! and write `Success(v)` / `Error { error, .. }` / `Defect { defect, .. }`
//! unqualified. `Error` is an *expected* failure (a typed vox/app error);
//! `Defect` is an *unexpected* one (a panic/bug carried as data) — a page
//! can render the two differently.

/// Async state that keeps the last good value visible while the next
/// request is in flight (stale-while-revalidate).
///
/// `E` defaults to `String` because the example screens stringify vox
/// errors at the call site (`format!("{e:?}")`); a typed error still works
/// by naming the parameter.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AtomResult<T, E = String> {
    /// Never requested. No value yet.
    Initial,
    /// First load in flight. No value yet.
    Loading,
    /// A refetch is in flight, but the last good value is still shown.
    Reloading(T),
    /// The last request succeeded.
    Success(T),
    /// An expected, typed failure (a vox/app error). `last` is the stale
    /// value to keep showing, if any.
    Error {
        /// The error from the failed request.
        error: E,
        /// The last good value, retained for stale-while-revalidate.
        last: Option<T>,
    },
    /// An unexpected failure — a panic or bug, carried as data. `last` is
    /// the stale value to keep showing, if any.
    Defect {
        /// The defect message.
        defect: String,
        /// The last good value, retained for stale-while-revalidate.
        last: Option<T>,
    },
}

// Manual, not derived: `#[derive(Default)]` would over-bound the impl with
// `T: Default, E: Default`, but `Initial` holds neither.
#[allow(clippy::derivable_impls)]
impl<T, E> Default for AtomResult<T, E> {
    fn default() -> Self {
        Self::Initial
    }
}

impl<T, E> AtomResult<T, E> {
    /// The best value available in any phase (stale-while-revalidate): the
    /// success/reloading value, or a retained stale value on failure.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Reloading(v) | Self::Success(v) => Some(v),
            Self::Error { last: Some(v), .. } | Self::Defect { last: Some(v), .. } => Some(v),
            _ => None,
        }
    }

    /// The typed error of the most recent failed request (not a defect).
    pub fn error(&self) -> Option<&E> {
        match self {
            Self::Error { error, .. } => Some(error),
            _ => None,
        }
    }

    /// The defect message of the most recent failed request (not an error).
    pub fn defect(&self) -> Option<&str> {
        match self {
            Self::Defect { defect, .. } => Some(defect),
            _ => None,
        }
    }

    /// True while a request is in flight — first load *or* refetch.
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading | Self::Reloading(_))
    }

    /// True if the most recent request succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    /// True before any request has been made.
    pub fn is_initial(&self) -> bool {
        matches!(self, Self::Initial)
    }

    /// True while *waiting* on a request: no request has settled yet
    /// (`Initial`) or one is in flight (`Loading`/`Reloading`).
    pub fn is_waiting(&self) -> bool {
        matches!(self, Self::Initial | Self::Loading | Self::Reloading(_))
    }

    /// Combine two results into one all-or-nothing phase. Precedence:
    /// `Defect` > `Error` > waiting; `Reloading` only when **both** sides
    /// still have a value to show. Use when a page can't render until
    /// several resources are ready:
    ///
    /// ```ignore
    /// match use_example(id).zip(use_invoice(id)) {
    ///     Success((example, invoice)) => rsx! { Combined { example, invoice } },
    ///     /* one Spinner / StatusError set for the pair */
    /// }
    /// ```
    pub fn zip<U>(self, other: AtomResult<U, E>) -> AtomResult<(T, U), E> {
        use AtomResult::*;
        // Failures dominate (carrying any stale pair as `last`)…
        let pair_of = |a: Option<T>, b: Option<U>| Some((a?, b?));
        match (self, other) {
            (Defect { defect, last }, other) => {
                let last = pair_of(last, other.into_value());
                Defect { defect, last }
            }
            (this, Defect { defect, last }) => {
                let last = pair_of(this.into_value(), last);
                Defect { defect, last }
            }
            (Error { error, last }, other) => {
                let last = pair_of(last, other.into_value());
                Error { error, last }
            }
            (this, Error { error, last }) => {
                let last = pair_of(this.into_value(), last);
                Error { error, last }
            }
            // …then full success…
            (Success(a), Success(b)) => Success((a, b)),
            // …then in-flight with both values still showable…
            (a, b) => {
                let initial = a.is_initial() && b.is_initial();
                match (a.into_value(), b.into_value()) {
                    (Some(a), Some(b)) => Reloading((a, b)),
                    _ if initial => Initial,
                    _ => Loading,
                }
            }
        }
    }

    /// Combine many same-typed results: `Success` only when every input
    /// is. Same precedence as [`zip`](Self::zip).
    pub fn all(results: impl IntoIterator<Item = AtomResult<T, E>>) -> AtomResult<Vec<T>, E> {
        let mut acc = AtomResult::Success(Vec::new());
        for r in results {
            acc = acc.zip(r).map(|(mut v, t)| {
                v.push(t);
                v
            });
        }
        acc
    }

    /// The best value in any phase, by value (`value()`'s owning sibling).
    fn into_value(self) -> Option<T> {
        match self {
            Self::Reloading(v) | Self::Success(v) => Some(v),
            Self::Error { last, .. } | Self::Defect { last, .. } => last,
            _ => None,
        }
    }

    /// Transform the contained value, preserving the phase.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> AtomResult<U, E> {
        match self {
            Self::Initial => AtomResult::Initial,
            Self::Loading => AtomResult::Loading,
            Self::Reloading(v) => AtomResult::Reloading(f(v)),
            Self::Success(v) => AtomResult::Success(f(v)),
            Self::Error { error, last } => AtomResult::Error {
                error,
                last: last.map(f),
            },
            Self::Defect { defect, last } => AtomResult::Defect {
                defect,
                last: last.map(f),
            },
        }
    }
}

impl<T: Clone, E> AtomResult<T, E> {
    /// Move into a refetch, keeping any current value visible
    /// (`Success`/`Reloading`/stale-failure → `Reloading`, else `Loading`).
    /// Call this as the backing fetch *starts*, in an effect — never during
    /// render.
    #[must_use]
    pub fn begin_reload(self) -> Self {
        match self {
            Self::Success(v) | Self::Reloading(v) => Self::Reloading(v),
            Self::Error { last: Some(v), .. } | Self::Defect { last: Some(v), .. } => {
                Self::Reloading(v)
            }
            _ => Self::Loading,
        }
    }

    /// Fold a finished fetch outcome in: `Ok` → `Success`, `Err` → `Error`
    /// retaining the prior value as `last`. Call this as the backing fetch
    /// *settles*, in an effect — never during render.
    #[must_use]
    pub fn settle(self, outcome: Result<T, E>) -> Self {
        match outcome {
            Ok(v) => Self::Success(v),
            Err(error) => {
                let last = self.value().cloned();
                Self::Error { error, last }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type R = AtomResult<i32, String>;

    #[test]
    fn value_is_available_in_every_value_bearing_phase() {
        assert_eq!(R::Initial.value(), None);
        assert_eq!(R::Loading.value(), None);
        assert_eq!(R::Reloading(7).value(), Some(&7));
        assert_eq!(R::Success(7).value(), Some(&7));
        assert_eq!(
            R::Error {
                error: "x".into(),
                last: Some(7)
            }
            .value(),
            Some(&7)
        );
        assert_eq!(
            R::Error {
                error: "x".into(),
                last: None
            }
            .value(),
            None
        );
    }

    #[test]
    fn reload_keeps_previous_value() {
        assert_eq!(R::Success(7).begin_reload(), R::Reloading(7));
        assert_eq!(R::Initial.begin_reload(), R::Loading);
        assert_eq!(
            R::Error {
                error: "x".into(),
                last: Some(7)
            }
            .begin_reload(),
            R::Reloading(7)
        );
    }

    #[test]
    fn settle_failure_retains_last_good_value() {
        let prior = R::Reloading(7);
        let settled = prior.settle(Err("boom".into()));
        assert_eq!(settled.value(), Some(&7));
        assert_eq!(settled.error(), Some(&"boom".to_string()));
        assert!(!settled.is_loading());
    }

    #[test]
    fn settle_ok_replaces_value() {
        assert_eq!(R::Reloading(7).settle(Ok(9)), R::Success(9));
    }

    #[test]
    fn error_and_defect_are_distinct_channels() {
        let err = R::Initial.settle(Err("nope".into()));
        assert_eq!(err.error(), Some(&"nope".to_string()));
        assert_eq!(err.defect(), None);

        let def = R::Defect {
            defect: "panicked".into(),
            last: None,
        };
        assert_eq!(def.defect(), Some("panicked"));
        assert_eq!(def.error(), None);
    }

    #[test]
    fn zip_combines_phases_with_failure_precedence() {
        use AtomResult::*;
        // both success
        assert_eq!(
            R::Success(1).zip(AtomResult::<i32, String>::Success(2)),
            Success((1, 2))
        );
        // error dominates, stale pair retained when both sides have values
        let z = R::Error {
            error: "e".into(),
            last: Some(1),
        }
        .zip(AtomResult::<i32, String>::Success(2));
        assert_eq!(
            z,
            Error {
                error: "e".into(),
                last: Some((1, 2))
            }
        );
        // defect beats error
        let z = R::Error {
            error: "e".into(),
            last: None,
        }
        .zip(AtomResult::<i32, String>::Defect {
            defect: "d".into(),
            last: None,
        });
        assert!(matches!(z, Defect { .. }));
        // one side loading without value -> Loading; both initial -> Initial
        assert_eq!(
            R::Success(1).zip(AtomResult::<i32, String>::Loading),
            Loading
        );
        assert_eq!(R::Initial.zip(AtomResult::<i32, String>::Initial), Initial);
        // both sides hold values, one refetching -> Reloading(pair)
        assert_eq!(
            R::Reloading(1).zip(AtomResult::<i32, String>::Success(2)),
            Reloading((1, 2))
        );
    }

    #[test]
    fn all_collects_or_degrades() {
        let ok = AtomResult::all([R::Success(1), R::Success(2), R::Success(3)]);
        assert_eq!(ok, AtomResult::Success(vec![1, 2, 3]));
        let degraded = AtomResult::all([R::Success(1), R::Loading]);
        assert_eq!(degraded, AtomResult::Loading);
    }

    #[test]
    fn waiting_covers_initial_and_in_flight_phases() {
        assert!(R::Initial.is_waiting());
        assert!(R::Loading.is_waiting());
        assert!(R::Reloading(7).is_waiting());
        assert!(!R::Success(7).is_waiting());
        assert!(
            !R::Error {
                error: "e".into(),
                last: None
            }
            .is_waiting()
        );
    }
}
