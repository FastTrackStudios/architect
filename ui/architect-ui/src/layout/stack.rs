//! VStack and HStack — flex containers with configurable gap.
//!
//! Uses inline `style` for gap instead of Tailwind classes to avoid
//! the dynamic class name issue where Tailwind can't see `gap-{n}`
//! at build time.

use dioxus::prelude::*;

/// Convert a Tailwind spacing value to rem.
/// "0" -> "0", "0.5" -> "0.125rem", "1" -> "0.25rem", "2" -> "0.5rem", etc.
fn spacing_to_rem(value: &str) -> String {
    if value == "0" {
        return "0".into();
    }
    match value.parse::<f64>() {
        Ok(n) => format!("{}rem", n * 0.25),
        Err(_) => "0.5rem".into(), // fallback
    }
}

/// Vertical flex container (column direction).
#[derive(Props, Clone, PartialEq)]
pub struct VStackProps {
    /// Tailwind spacing value (e.g. "2", "4", "6"). Defaults to "2".
    #[props(default = "2".to_string())]
    pub gap: String,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn VStack(props: VStackProps) -> Element {
    let gap_rem = spacing_to_rem(&props.gap);
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col", props.class.as_str()]),
            style: "gap: {gap_rem};",
            {props.children}
        }
    }
}

/// Horizontal flex container (row direction).
#[derive(Props, Clone, PartialEq)]
pub struct HStackProps {
    /// Tailwind spacing value (e.g. "2", "4", "6"). Defaults to "2".
    #[props(default = "2".to_string())]
    pub gap: String,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn HStack(props: HStackProps) -> Element {
    let gap_rem = spacing_to_rem(&props.gap);
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-row items-center", props.class.as_str()]),
            style: "gap: {gap_rem};",
            {props.children}
        }
    }
}
