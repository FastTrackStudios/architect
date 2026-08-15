//! Core, dependency-light types for `architect-story`.
//!
//! This crate intentionally does NOT depend on `dioxus`. It defines:
//!
//! - [`Story`] — the metadata + render-fn pointer that `#[story]` produces.
//! - [`Knob`] / [`KnobValue`] — typed inputs to a story.
//! - [`Interaction`] — a step in a recorded interaction script (click, type,
//!   key press, scroll, wait, snapshot, assert).
//! - [`STORIES`] — the global, compile-time registry populated via
//!   [`linkme::distributed_slice`]. Both the interactive shell and the
//!   headless VRT runner enumerate stories from this slice.
//!
//! Renderer-specific concerns (rendering Dioxus VDOMs, rasterizing via
//! Blitz, dispatching events into a `DioxusDocument`) live in
//! `architect-story-runtime` and `architect-story-snapshots`. Anything that needs to talk about
//! a story without pulling in Dioxus belongs here.

#![cfg_attr(docsrs, feature(doc_cfg))]

// linkme 0.3 doesn't support `target_os = "unknown"` (i.e.
// wasm32-unknown-unknown). Apps that consume `architect-ui` from a browser
// wasm target still need the rest of architect-ui to compile, so on wasm32
// the registry collapses to an empty array. Story enumeration only
// runs in the native shell / VRT runner anyway.
#[cfg(not(target_arch = "wasm32"))]
use linkme::distributed_slice;

/// The compile-time story registry.
///
/// `#[story]` (in `architect-story-macros`) appends to this slice. Tools that
/// enumerate stories — the interactive shell, the VRT runner, the fuzz
/// harness — iterate it.
///
/// ```ignore
/// for story in architect_story_core::STORIES {
///     println!("{}/{}", story.category, story.name);
/// }
/// ```
#[cfg(not(target_arch = "wasm32"))]
#[distributed_slice]
pub static STORIES: [&'static Story] = [..];

#[cfg(target_arch = "wasm32")]
pub static STORIES: [&'static Story; 0] = [];

/// Metadata + render entry point for a single story.
///
/// `render` is intentionally an opaque pointer — its signature is defined
/// by `architect-story-runtime`. This crate stores it as `*const ()` so we can
/// keep `architect-story-core` free of any rendering dependency.
pub struct Story {
    /// Stable identifier — usually the function name.
    pub name: &'static str,
    /// Sidebar grouping. `None` collapses into a default "Uncategorised".
    pub category: Option<&'static str>,
    /// Source-doc-comment description, surfaced in the shell.
    pub description: &'static str,
    /// Rust path to the source function (file:line, populated by the macro).
    pub source: &'static str,
    /// Knob declarations, in declaration order.
    pub knobs: &'static [KnobSpec],
    /// Render thunk. Cast back to the runtime-defined fn pointer.
    pub render: *const (),
    /// Optional auto-snapshot matrix. When present, the VRT runner takes
    /// the cartesian product of the listed knob values and snapshots each.
    /// `None` means "use defaults only".
    pub auto_states: Option<&'static [StateAssignment]>,
}

// SAFETY: the `render` pointer is to a `'static` fn item; sharing across
// threads is fine. Stories themselves are immutable `&'static`.
unsafe impl Sync for Story {}

// `Story` is identity-compared so it can flow through Dioxus props
// (which require `PartialEq`). Two stories are equal iff they share the
// same `&'static` address — story values are interned at link time, so
// pointer equality matches semantic equality without forcing every
// nested field to derive PartialEq.
impl PartialEq for Story {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}
impl Eq for Story {}

/// One knob declaration.
#[derive(Clone)]
pub struct KnobSpec {
    pub name: &'static str,
    pub doc: &'static str,
    pub kind: KnobKind,
    /// The default value used when no `KnobSource` override is present.
    /// `None` means the knob is "opaque" (the shell can't synthesise a
    /// default — typically because the underlying type is a non-primitive
    /// the macro couldn't introspect). The macro emits the user's
    /// `#[knob(default = ...)]` expression directly into the render
    /// thunk in that case, so a missing `default` here doesn't break
    /// rendering — just hides the knob from the shell editor.
    pub default: Option<KnobValue>,
}

// Identity-compare so `&'static KnobSpec` flows through Dioxus props.
// Each `#[story]` emits its KnobSpec values into a single `static`
// slice, so pointer equality ≡ semantic equality.
impl PartialEq for KnobSpec {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}
impl Eq for KnobSpec {}

/// What kind of input a knob accepts. The shell uses this to pick a
/// control widget; the VRT runner uses it to enumerate values for the
/// auto-state matrix.
#[derive(Clone)]
pub enum KnobKind {
    Bool,
    /// Variants of an enum. The first is the default unless overridden.
    Enum {
        variants: &'static [&'static str],
    },
    String {
        multiline: bool,
    },
    Number {
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    },
    Color,
    /// Opaque value the shell can't introspect. Knob shows as read-only.
    Opaque,
}

/// Either a literal default or a `serde_json`-style override emitted by
/// `#[states(...)]`. Stored as `&'static str` so this stays alloc-free.
pub struct StateAssignment {
    pub name: &'static str,
    pub knobs: &'static [(&'static str, KnobValue)],
}

/// A static, statically-typed knob value. The macro converts literals
/// into these.
#[derive(Clone, Debug)]
pub enum KnobValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'static str),
    /// One variant of an enum, by name. The runtime is responsible for
    /// converting the name back into the actual enum value via
    /// `<T as FromKnobName>::from_name`.
    EnumVariant(&'static str),
}

/// One step in an interaction script.
///
/// Used by `architect-story-snapshots` to drive a `DioxusDocument` between snapshots,
/// and by `architect-story-fuzz` as the unit of mutation.
pub enum Interaction {
    /// Click the first node matching the selector.
    Click(Selector),
    /// Hover (mousemove + mouseenter) onto the matching node.
    Hover(Selector),
    /// Press and release a key on the currently-focused node.
    Key(KeyAction),
    /// Type a string into the currently-focused node.
    Type(&'static str),
    /// Scroll the matching node by (dx, dy) pixels.
    Scroll { target: Selector, dx: f32, dy: f32 },
    /// Wait for a condition before continuing.
    Wait(WaitCondition),
    /// Take a named snapshot at this point in the script.
    Snapshot(&'static str),
    /// Assert against the running app via a runtime-defined predicate.
    /// The opaque pointer is interpreted by `architect-story-runtime`.
    Assert(*const ()),
}

unsafe impl Sync for Interaction {}

/// How to find a node in the rendered document.
pub enum Selector {
    /// CSS-style selector evaluated by Blitz/Stylo.
    Css(&'static str),
    /// ARIA role + accessible-name pair (preferred — survives styling churn).
    Role {
        role: &'static str,
        name: Option<&'static str>,
    },
    /// `data-test-id` attribute.
    TestId(&'static str),
}

pub enum KeyAction {
    Press(&'static str),
    Down(&'static str),
    Up(&'static str),
}

pub enum WaitCondition {
    /// The runtime is idle (no pending mutations or pending futures).
    Idle,
    /// A node matching the selector has mounted.
    Mounted(Selector),
    /// A fixed wall-clock duration. Discouraged — prefer `Idle`/`Mounted`.
    Millis(u64),
}

/// Recorded interaction script associated with a story.
pub struct InteractionScript {
    pub story: &'static str,
    pub name: &'static str,
    pub steps: &'static [Interaction],
}

#[cfg(not(target_arch = "wasm32"))]
#[distributed_slice]
pub static INTERACTION_SCRIPTS: [&'static InteractionScript] = [..];

#[cfg(target_arch = "wasm32")]
pub static INTERACTION_SCRIPTS: [&'static InteractionScript; 0] = [];
