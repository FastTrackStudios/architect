//! Runtime support for `#[architect::rpc(ops)]` — resolving deferred
//! op arguments.
//!
//! `ops(Src as Dst, ...)` substitution pairs let a reified op carry a
//! *deferred* argument representation (`Dst`) on the wire — e.g. "the
//! track produced by step 3" instead of a literal `TrackRef`. At
//! `apply` time the emitted code hands each deferred value to the
//! caller-supplied resolver:
//!
//! ```ignore
//! struct StepOutputs { /* results of earlier batch steps */ }
//!
//! impl OpResolver for StepOutputs {
//!     type Error = BatchError;
//! }
//!
//! impl ResolveArg<TrackArg, TrackRef> for StepOutputs {
//!     fn resolve_arg(&self, arg: TrackArg) -> Result<TrackRef, BatchError> {
//!         match arg {
//!             TrackArg::Literal(t) => Ok(t),
//!             TrackArg::FromStep(n) => self.track_from(n),
//!         }
//!     }
//! }
//! ```
//!
//! One resolver serves every substituted pair on a trait; the single
//! `Error` associated type lives on [`OpResolver`] so `apply` returns
//! one error type regardless of how many pairs are declared.

/// A resolver's error channel — supertrait of every [`ResolveArg`]
/// impl so one `apply` call has one error type.
pub trait OpResolver {
    /// Error produced when a deferred argument cannot be resolved
    /// (missing step, type mismatch, out-of-range index, ...).
    type Error;
}

/// Convert one deferred wire representation `A` back into the method
/// parameter type `T` the backend call needs.
pub trait ResolveArg<A, T>: OpResolver {
    /// Resolve `arg`, typically by looking up earlier step outputs.
    fn resolve_arg(&self, arg: A) -> Result<T, Self::Error>;
}

/// Resolver for programs that only ever carry literal arguments —
/// useful when the deferred representation has a trivial `Into<T>`
/// conversion and cross-step references are not in play.
#[derive(Debug, Default, Clone, Copy)]
pub struct LiteralResolver;

impl OpResolver for LiteralResolver {
    type Error = core::convert::Infallible;
}

impl<A, T> ResolveArg<A, T> for LiteralResolver
where
    A: Into<T>,
{
    fn resolve_arg(&self, arg: A) -> Result<T, Self::Error> {
        Ok(arg.into())
    }
}
