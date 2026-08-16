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

/// The canonical FTS theme stylesheet, embedded as a string.
///
/// Consumers in other repositories cannot `include_str!` this out of the
/// crate's `assets/` directory — a git dependency has no stable path on
/// disk — so the bytes are embedded here instead. Use this anywhere the
/// old `include_str!(".../libs/fts-ui/fts-ui/assets/fts-theme.css")` was
/// used before the monorepo split.
pub const THEME_CSS: &str = include_str!("../assets/fts-theme.css");

/// The compiled Tailwind utilities this crate's components actually use.
///
/// Consumers cannot generate these themselves. Tailwind's `@source` resolves
/// on the filesystem, and a git dependency has no stable path — so a
/// downstream sheet silently omits every class used only inside architect-ui
/// (Button/Sheet/Badge variants and the rest). Silently is the operative
/// word: a `@source` matching nothing does not error, it just yields fewer
/// classes and an element that renders unstyled.
///
/// Inject this alongside [`THEME_CSS`]:
///
/// ```rust,ignore
/// document::Style { {architect_ui::THEME_CSS} }
/// document::Style { {architect_ui::UTILITIES_CSS} }
/// ```
///
/// Regenerate with `just ui-css` after adding classes to a component.
pub const UTILITIES_CSS: &str = include_str!("../assets/utilities.css");

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
