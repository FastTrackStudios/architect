//! Interaction fuzzing.
//!
//! Stub. Plan:
//!
//! - For each story (or filtered subset), spin up a `DioxusDocument` via
//!   `architect-story-snapshots`, enumerate focusable / clickable nodes, and apply a
//!   `proptest`-generated sequence of `Interaction` steps.
//! - Assertions: no panics, no `tracing::error!`, optional user
//!   invariants registered via `#[story_test]`.
//! - On failure, persist the minimised step sequence + a screenshot for
//!   triage.

#![cfg(not(debug_assertions))]
