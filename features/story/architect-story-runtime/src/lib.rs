//! Dioxus-aware runtime for `architect-story`.
//!
//! Re-exports `architect-story-core` and `architect-story-macros` so consumers only need
//! one dep, and provides:
//!
//! - The concrete render-thunk signature stored in
//!   [`Story::render`](architect_story_core::Story::render). The opaque
//!   `*const ()` in core is cast back to [`RenderFn`] by callers.
//! - [`KnobSource`] — runtime trait the macro-generated thunk uses to
//!   read knob values out of the shell or VRT runner.
//! - Helpers for executing [`Interaction`](architect_story_core::Interaction)
//!   steps against a `dioxus_core::VirtualDom` (and, in
//!   `architect-story-snapshots`, a `DioxusDocument`).
//!
//! This crate intentionally has no Blitz / wgpu dep. It works in any
//! Dioxus 0.7 context — desktop, web, native — so the interactive
//! shell can run anywhere.

pub use architect_story_core::*;
pub use architect_story_macros::{states, story, story_test};

// Re-export `linkme` so the `#[story]` macro can emit
// `::architect_story_runtime::linkme::distributed_slice(...)` and consumers
// don't need a direct `linkme` dep.
pub use linkme;

use dioxus::prelude::Element;

/// Concrete signature stored in [`Story::render`].
///
/// `knobs` is provided by the caller (the shell's knob editor, the VRT
/// runner's matrix iterator, the fuzz harness's mutator). The macro
/// generates a thunk that reads each declared knob out of it via
/// [`KnobSource::get`] and invokes the user-written component fn.
pub type RenderFn = fn(&dyn KnobSource) -> Element;

/// Source of typed knob values for a single render.
///
/// The shell implements this against a `Signal<HashMap<&'static str, Knob
/// Value>>`; the VRT runner implements it against a fixed
/// `&[(&'static str, KnobValue)]` from the state matrix.
pub trait KnobSource {
    fn get(&self, name: &'static str) -> Option<&KnobValue>;
}

/// Convenience — read a knob with a fallback default.
pub fn knob<'a>(
    src: &'a dyn KnobSource,
    name: &'static str,
    fallback: &'a KnobValue,
) -> &'a KnobValue {
    src.get(name).unwrap_or(fallback)
}

/// Cast a `Story::render` opaque pointer back to a callable.
///
/// # Safety
/// `story.render` must have been produced by the `#[story]` macro (or a
/// hand-written story that points at a `RenderFn`). Mismatched signatures
/// are UB.
#[inline]
pub unsafe fn render_fn(story: &Story) -> RenderFn {
    unsafe { std::mem::transmute::<*const (), RenderFn>(story.render) }
}

/// Zero-sized [`KnobSource`] for stories that take no knobs. Useful while
/// hand-rolling stories before the `#[story]` macro lands.
pub struct NoKnobs;

impl KnobSource for NoKnobs {
    #[inline]
    fn get(&self, _name: &'static str) -> Option<&KnobValue> {
        None
    }
}

/// Helper for hand-rolling stories. Wraps `Story` construction so callers
/// don't have to remember to set every field. The `render` field is built
/// from a `RenderFn` and cast to `*const ()` here.
///
/// ```ignore
/// pub static BUTTON_PRIMARY: Story = const_story(StoryDef {
///     name: "button_primary",
///     category: Some("Buttons"),
///     description: "Default primary button.",
///     source: concat!(file!(), ":", line!()),
///     render: |_| rsx! { Button { "Click me" } },
///     ..StoryDef::DEFAULT
/// });
///
/// #[linkme::distributed_slice(STORIES)]
/// static __BUTTON_PRIMARY_REG: &Story = &BUTTON_PRIMARY;
/// ```
pub struct StoryDef {
    pub name: &'static str,
    pub category: Option<&'static str>,
    pub description: &'static str,
    pub source: &'static str,
    pub render: RenderFn,
    pub knobs: &'static [KnobSpec],
    pub auto_states: Option<&'static [StateAssignment]>,
}

impl StoryDef {
    pub const DEFAULT: Self = Self {
        name: "",
        category: None,
        description: "",
        source: "",
        render: |_| panic!("StoryDef::DEFAULT render thunk invoked"),
        knobs: &[],
        auto_states: None,
    };
}

/// `const fn` builder so stories can be `static` items.
pub const fn const_story(def: StoryDef) -> Story {
    Story {
        name: def.name,
        category: def.category,
        description: def.description,
        source: def.source,
        knobs: def.knobs,
        render: def.render as *const (),
        auto_states: def.auto_states,
    }
}
