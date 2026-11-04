//! The mark a pane shows while its view is busy: a frame of an
//! extension's view is running past its budget, so the pane keeps the
//! last good frame and says so (`docs/content/architecture/extensions.md`,
//! "Responsive by design: views and workers").

use dioxus::prelude::*;

use super::{ShellTheme, try_use_shell};

/// A small pill in the top right corner of its (positioned) parent: an
/// accent dot and `label` ("Busy" when none), in the shell's theme. It
/// takes no pointer, so the pane under it still answers.
#[component]
pub fn BusyBadge(label: Option<String>) -> Element {
    let t = try_use_shell().map_or_else(ShellTheme::default, |s| s.theme);
    let label = label.unwrap_or_else(|| "Busy".to_owned());
    rsx! {
        div {
            class: "architect-busy-badge",
            style: "position: absolute; top: 8px; right: 8px; z-index: 5; pointer-events: none; \
                    display: flex; align-items: center; gap: 6px; padding: 3px 9px; \
                    border-radius: 999px; border: 1px solid {t.rule}; background: {t.pill}; \
                    box-shadow: {t.shadow_lift}; color: {t.ink_2}; font-family: {t.font}; \
                    font-size: 11px; line-height: 16px;",
            div { style: "width: 7px; height: 7px; border-radius: 50%; background: {t.accent};" }
            "{label}"
        }
    }
}
