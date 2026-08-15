//! Resizable — shadcn v4 maia style resizable panel layout.
//!
//! Simplified flex-basis approach (no JS drag interop).

use architect_story_runtime::story;
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// Direction
// ---------------------------------------------------------------------------

/// Layout direction for a resizable panel group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizeDirection {
    #[default]
    Horizontal,
    Vertical,
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// Shared context provided by [`ResizablePanelGroup`] to children.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResizableContext {
    pub direction: ResizeDirection,
}

// ---------------------------------------------------------------------------
// ResizablePanelGroup
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct ResizablePanelGroupProps {
    /// Layout direction (default: Horizontal).
    #[props(default)]
    pub direction: ResizeDirection,

    /// Extra CSS classes on the outer container.
    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// Container for resizable panels and handles.
#[component]
pub fn ResizablePanelGroup(props: ResizablePanelGroupProps) -> Element {
    let base = match props.direction {
        ResizeDirection::Horizontal => "flex h-full w-full",
        ResizeDirection::Vertical => "flex flex-col h-full w-full",
    };

    use_context_provider(|| ResizableContext {
        direction: props.direction,
    });

    rsx! {
        div {
            class: crate::cn::merge_slice(&[base, props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// ResizablePanel
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct ResizablePanelProps {
    /// Default size as a percentage (e.g. 50.0 for 50%).
    #[props(default = 50.0)]
    pub default_size: f64,

    /// Minimum size as a percentage.
    #[props(default)]
    pub min_size: Option<f64>,

    /// Maximum size as a percentage.
    #[props(default)]
    pub max_size: Option<f64>,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// A single panel inside a [`ResizablePanelGroup`].
#[component]
pub fn ResizablePanel(props: ResizablePanelProps) -> Element {
    let ctx: ResizableContext = use_context();

    let mut style = format!(
        "flex-basis: {}%; flex-shrink: 0; flex-grow: 0;",
        props.default_size
    );

    if let Some(min) = props.min_size {
        match ctx.direction {
            ResizeDirection::Horizontal => {
                style.push_str(&format!(" min-width: {min}%;"));
            }
            ResizeDirection::Vertical => {
                style.push_str(&format!(" min-height: {min}%;"));
            }
        }
    }

    if let Some(max) = props.max_size {
        match ctx.direction {
            ResizeDirection::Horizontal => {
                style.push_str(&format!(" max-width: {max}%;"));
            }
            ResizeDirection::Vertical => {
                style.push_str(&format!(" max-height: {max}%;"));
            }
        }
    }

    rsx! {
        div {
            class: crate::cn::merge_slice(&["overflow-auto", props.class.as_str()]),
            style: "{style}",
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// ResizableHandle
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct ResizableHandleProps {
    /// Show a visible grip icon in the centre of the handle.
    #[props(default = false)]
    pub with_handle: bool,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,
}

/// Visual drag handle between two [`ResizablePanel`]s.
///
/// Note: actual drag-resize behaviour would require JS interop. This component
/// renders the correct visual affordance only.
#[component]
pub fn ResizableHandle(props: ResizableHandleProps) -> Element {
    let ctx: ResizableContext = use_context();

    let base = "relative flex shrink-0 items-center justify-center \
                after:absolute after:inset-y-0 after:left-1/2 after:-translate-x-1/2";

    let dir_cls = match ctx.direction {
        ResizeDirection::Horizontal => "w-px bg-border cursor-col-resize after:w-4",
        ResizeDirection::Vertical => "h-px bg-border cursor-row-resize after:h-4",
    };

    rsx! {
        div {
            class: crate::cn::merge_slice(&[base, dir_cls, props.class.as_str()]),

            if props.with_handle {
                div {
                    class: "z-10 flex h-4 w-3 items-center justify-center rounded-sm border bg-border",
                    // Grip dots rendered as an SVG matching shadcn's GripVertical icon.
                    svg {
                        xmlns: "http://www.w3.org/2000/svg",
                        width: "10",
                        height: "14",
                        view_box: "0 0 10 14",
                        fill: "currentColor",
                        // 6 small circles arranged as a 2×3 grip pattern
                        circle { cx: "3", cy: "3", r: "1" }
                        circle { cx: "7", cy: "3", r: "1" }
                        circle { cx: "3", cy: "7", r: "1" }
                        circle { cx: "7", cy: "7", r: "1" }
                        circle { cx: "3", cy: "11", r: "1" }
                        circle { cx: "7", cy: "11", r: "1" }
                    }
                }
            }
        }
    }
}

/// Default Resizable story rendering two horizontal panels with a handle.
#[story(category = "Resizable", name = "default")]
pub fn resizable_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            div { class: "h-64 w-full max-w-2xl rounded-lg border border-border overflow-hidden",
                ResizablePanelGroup { direction: ResizeDirection::Horizontal,
                    ResizablePanel { default_size: 40.0, class: "bg-muted/30".to_string(),
                        div { class: "flex h-full items-center justify-center text-sm text-muted-foreground",
                            "Sidebar"
                        }
                    }
                    ResizableHandle { with_handle: true }
                    ResizablePanel { default_size: 60.0,
                        div { class: "flex h-full items-center justify-center text-sm text-muted-foreground",
                            "Main"
                        }
                    }
                }
            }
        }
    }
}
