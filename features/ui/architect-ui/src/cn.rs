//! Tailwind class composition utilities.
//!
//! `cn!` is the Rust equivalent of shadcn's `cn(...)` helper:
//! `twMerge(clsx(...inputs))`.
//!
//! ```rust,ignore
//! let active = true;
//! let class = architect_ui::cn!(
//!     "rounded-md px-2 py-1",
//!     (active, "bg-primary text-primary-foreground"),
//!     Some("py-2"),
//! );
//! assert_eq!(class, "rounded-md px-2 bg-primary text-primary-foreground py-2");
//! ```
//!
/// Merge an already-joined Tailwind class string.
///
/// Prefer [`cn!`] at component call sites. This helper is for lower-level code
/// that already has a single class string.
pub fn merge(input: impl AsRef<str>) -> String {
    tw_merge::merge::merge_classes(input)
}

/// Merge a slice of Tailwind class strings without first collecting them.
///
/// Prefer [`cn!`] at component call sites. This helper is useful when class
/// fragments are already represented as a slice.
pub fn merge_slice(inputs: &[&str]) -> String {
    tw_merge::merge::tw_merge_slice(inputs)
}

/// Compose classes with `clsx`, then resolve Tailwind conflicts with `tw_merge`.
///
/// Supported inputs are the inputs accepted by the `clsx` crate: strings,
/// arrays/slices, `Option<T>`, `(bool, T)` conditionals, maps, numbers, and
/// closures returning another `clsx` argument.
#[macro_export]
macro_rules! cn {
    () => {
        String::new()
    };
    ($($input:expr),+ $(,)?) => {{
        let classes = ::clsx::clsx!($($input),+);
        ::tw_merge::merge::merge_classes(classes)
    }};
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn merge_helpers_resolve_basic_conflicts() {
        assert_eq!(merge("py-6 py-2"), "py-2");
        assert_eq!(merge_slice(&["bg-card", "bg-red-500"]), "bg-red-500");
    }

    #[test]
    fn merge_helpers_preserve_refinements() {
        assert_eq!(merge_slice(&["p-4", "py-2"]), "p-4 py-2");
        assert_eq!(merge_slice(&["px-6 py-6", "px-0 py-2"]), "px-0 py-2");
    }

    #[test]
    fn merge_helpers_preserve_non_conflicting_classes() {
        assert_eq!(
            merge_slice(&["bg-card ring-1", "py-2"]),
            "bg-card ring-1 py-2"
        );
        assert_eq!(merge_slice(&["", "px-4"]), "px-4");
    }

    #[test]
    fn merge_helpers_keep_variant_conflicts_scoped() {
        assert_eq!(
            merge_slice(&["hover:bg-muted", "focus:bg-muted"]),
            "hover:bg-muted focus:bg-muted"
        );
        assert_eq!(
            merge_slice(&["hover:bg-muted", "hover:bg-accent"]),
            "hover:bg-accent"
        );
    }

    #[test]
    fn macro_supports_clsx_style_conditionals() {
        let active = true;
        let disabled = false;

        assert_eq!(
            crate::cn!(
                "btn",
                (active, "bg-primary"),
                (disabled, "opacity-50"),
                Some("px-4")
            ),
            "btn bg-primary px-4"
        );
    }

    #[test]
    fn macro_supports_arrays_and_maps() {
        let mut classes = HashMap::new();
        classes.insert("rounded-md".to_string(), true);
        classes.insert("hidden".to_string(), false);

        assert_eq!(
            crate::cn!(["px-2", "py-1"], classes, "py-3"),
            "px-2 rounded-md py-3"
        );
    }
}
