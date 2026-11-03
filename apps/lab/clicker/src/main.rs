//! The lab's process extension (`apps/lab/clicker/extension.toml`): a
//! counter whose `VirtualDom` runs here, in a process of its own, and
//! whose edits the lab applies in a workspace pane. Started by the lab;
//! run by hand it says so and exits.
//!
//! It follows the app's theme two ways: its styles are the app's CSS
//! variables (`var(--foreground)`, and the clicker's own `--clicker-face`,
//! which the lab writes on `:root`), and the theme comes as data
//! (`guest::use_theme`), from which it works out colours of its own (the
//! ramp under the count).

use std::sync::atomic::{AtomicU64, Ordering};

use architect_extensions::guest::{Guest, GuestError, use_theme};
use dioxus::prelude::*;

static CLICKS: AtomicU64 = AtomicU64::new(0);

/// Steps of the ramp under the count.
const RAMP: u8 = 6;

#[component]
fn Counter() -> Element {
    let mut n = use_signal(|| 0_u64);
    let pid = std::process::id();
    let theme = use_theme();
    // Worked out here, from the theme as data: the count's colour faded
    // into the ground, step by step.
    let ramp: Vec<String> = theme.as_ref().map_or_else(Vec::new, |t| {
        let (from, to) = (t.color("clicker.count"), t.color("background"));
        (0..RAMP)
            .map(|i| from.mix(to, f32::from(i) / f32::from(RAMP)).to_css())
            .collect()
    });
    let radius = theme
        .as_ref()
        .map(|t| t.number("clicker.radius"))
        .filter(|r| r.is_finite())
        .unwrap_or(8.0);
    let about = theme.as_ref().map_or_else(
        || "No theme from the host yet.".to_owned(),
        |t| {
            format!(
                "Theme {} ({}), revision {}, as data from the host.",
                t.id(),
                t.scheme().as_str(),
                t.revision()
            )
        },
    );
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 12px; padding: 20px; \
                      font-family: -apple-system, system-ui, sans-serif; color: var(--foreground);",
            div { style: "font-size: 15px; font-weight: 650;", "Clicker" }
            div { style: "max-width: 420px; font-size: 13px; line-height: 1.5; color: var(--muted-foreground);",
                "An extension's view: drawn by its own process (number {pid}), which keeps the count. \
                 Stop that process and this pane says so; the lab runs on."
            }
            div { style: "font-size: 28px; font-weight: 700; color: var(--clicker-count);",
                {if n() == 1 { "1 click".to_owned() } else { format!("{n} clicks") }}
            }
            div { style: "display: flex; gap: 4px;",
                for (i, colour) in ramp.into_iter().enumerate() {
                    div { key: "{i}", style: "width: 28px; height: 10px; border-radius: 3px; background: {colour};" }
                }
            }
            button {
                style: "align-self: flex-start; padding: 6px 14px; border-radius: {radius}px; \
                        border: 1px solid var(--clicker-edge); background: var(--clicker-face); \
                        color: var(--foreground); font-size: 13px;",
                onclick: move |_| {
                    n += 1;
                    CLICKS.fetch_add(1, Ordering::Relaxed);
                },
                "Count a click"
            }
            div { style: "font-size: 12px; color: var(--muted-foreground);", "{about}" }
        }
    }
}

fn main() -> Result<(), GuestError> {
    Guest::new()
        .view("counter", Counter)
        .action("LAB_CLICKER_CLICKS", |_| async {
            Ok(serde_json::json!(CLICKS.load(Ordering::Relaxed)))
        })
        .serve()
}
