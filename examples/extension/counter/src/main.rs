//! A process extension (`examples/extension/counter/extension.toml`): a
//! counter whose `VirtualDom` runs here, in a process of its own. The
//! host starts this program with a socket in its environment; [`Guest`]
//! connects to it, says hello (the views and actions below), and from then
//! on renders each view the host opens into a stream of edits, and runs
//! each action the host is asked for. Run by hand, with no host, it says
//! so and exits.

use std::sync::atomic::{AtomicU64, Ordering};

use architect_extensions::guest::{Guest, GuestError};
use dioxus::prelude::*;

/// Clicks since the program started: the view counts them, the action
/// reads them. Gone with the process — a restarted counter starts at 0.
static CLICKS: AtomicU64 = AtomicU64::new(0);

/// The view `counter`: plain Dioxus. Its events (the click) come from the
/// host's pane, over the socket; what it changes goes back as edits.
#[component]
fn Counter() -> Element {
    let mut n = use_signal(|| 0_u64);
    let pid = std::process::id();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 8px; padding: 16px; font-family: system-ui, sans-serif;",
            p { id: "pid", "Drawn by process {pid}" }
            p { id: "count", "Clicked {n} times" }
            button {
                id: "more",
                style: "align-self: flex-start; width: 96px; height: 32px;",
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
        // Each `[[views]]` id of the manifest, and the component that draws it.
        .view("counter", Counter)
        // Each `[[actions]]` id, and what it does: JSON in, JSON out.
        .action("EXAMPLE_COUNTER_CLICKS", |_args| async {
            Ok(serde_json::json!(CLICKS.load(Ordering::Relaxed)))
        })
        .serve()
}
