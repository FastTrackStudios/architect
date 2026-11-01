//! The lab's process extension (`apps/lab/clicker/extension.toml`): a
//! counter whose `VirtualDom` runs here, in a process of its own, and
//! whose edits the lab applies in a workspace pane. Started by the lab;
//! run by hand it says so and exits.

use std::sync::atomic::{AtomicU64, Ordering};

use architect_extensions::guest::{Guest, GuestError};
use dioxus::prelude::*;

static CLICKS: AtomicU64 = AtomicU64::new(0);

#[component]
fn Counter() -> Element {
    let mut n = use_signal(|| 0_u64);
    let pid = std::process::id();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 12px; padding: 20px; \
                      font-family: -apple-system, system-ui, sans-serif; color: #e4e4e7;",
            div { style: "font-size: 12px; color: #a1a1aa;",
                "Drawn by process {pid}, over vox"
            }
            div { style: "font-size: 28px; font-weight: 700;", "{n}" }
            button {
                style: "align-self: flex-start; padding: 6px 14px; border-radius: 6px; \
                        border: 1px solid #3f3f46; background: #27272a; color: #e4e4e7; font-size: 13px;",
                onclick: move |_| {
                    n += 1;
                    CLICKS.fetch_add(1, Ordering::Relaxed);
                },
                "Click"
            }
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
