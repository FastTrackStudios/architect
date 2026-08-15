//! FastTrack Studio design system.
//!
//! Standalone UI component library for all FTS apps. Provides shadcn v4 maia
//! styled components, layout/typography primitives, and the canonical FTS
//! theme tokens.
//!
//! # Quick start
//! ```rust,ignore
//! use architect_ui::prelude::*;
//! ```

pub mod cn;
pub mod components;
pub mod layout;
pub mod theme;
pub mod typography;

pub mod prelude;
pub mod showcase;

pub mod stories;

// Re-export lucide icons for consumers.
pub use lucide_dioxus;

/// Accessible, unstyled primitives used as the behavior layer for FTS UI.
///
/// These are re-exported so applications and future FTS components can share
/// the same ARIA, focus, keyboard, and data-state behavior instead of each
/// component reimplementing it.
pub use dioxus_primitives as primitives;
