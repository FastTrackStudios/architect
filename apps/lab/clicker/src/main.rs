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
//!
//! It keeps its view responsive by the rule (`guest::use_work`): "Find
//! primes" counts the primes below ten million on a thread of its own,
//! its progress bar moving as it goes, the button still counting clicks;
//! "Block this view for 2 s" breaks the rule on purpose, sleeping on the
//! view's thread, so the lab marks the pane busy until it answers.

use std::sync::atomic::{AtomicU64, Ordering};

use architect_extensions::guest::{Guest, GuestError, Work, WorkStatus, host, use_theme, use_work};
use architect_extensions::hostcall::HostRequest;
use dioxus::prelude::*;

static CLICKS: AtomicU64 = AtomicU64::new(0);

/// Steps of the ramp under the count.
const RAMP: u8 = 6;

/// The primes below this many, counted off the view's thread.
const PRIMES_BELOW: u32 = 10_000_000;

/// Counts the primes below `n` the slow way, reporting as it goes.
fn count_primes(work: &Work, n: u32) -> Result<serde_json::Value, String> {
    let every = (n / 100).max(1);
    let mut found = 0_u32;
    for i in 2..n {
        let mut d = 2_u32;
        let mut prime = true;
        while d.saturating_mul(d) <= i {
            if i.is_multiple_of(d) {
                prime = false;
                break;
            }
            d = d.saturating_add(1);
        }
        if prime {
            found = found.saturating_add(1);
        }
        if i.is_multiple_of(every) {
            work.check()?;
            work.progress(f64::from(i), f64::from(n), format!("{found} primes so far"));
        }
    }
    Ok(serde_json::json!(found))
}

/// The heavy work, by the rule: a thread of its own, its progress here.
#[component]
fn Primes() -> Element {
    let primes = use_work();
    let state = primes.state();
    let pct = state.fraction() * 100.0;
    let said = match (&state.status, &state.result) {
        (WorkStatus::Done, Some(Ok(found))) => format!("{found} primes below {PRIMES_BELOW}"),
        (WorkStatus::Running, _) => state
            .progress
            .as_ref()
            .map_or_else(|| "Starting…".to_owned(), |p| p.note.clone()),
        (WorkStatus::Cancelled, _) => "Cancelled".to_owned(),
        (WorkStatus::Failed, Some(Err(why))) => format!("Failed: {why}"),
        _ => format!("Counts the primes below {PRIMES_BELOW} off this view's thread."),
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 8px; max-width: 420px;",
            div { style: "display: flex; gap: 8px;",
                if primes.running() {
                    button { class: "clicker-cancel", style: BUTTON, onclick: move |_| primes.cancel(), "Cancel" }
                } else {
                    button {
                        class: "clicker-primes",
                        style: BUTTON,
                        onclick: move |_| primes.start(|work| count_primes(work, PRIMES_BELOW)),
                        "Find primes"
                    }
                }
                button {
                    class: "clicker-block",
                    style: BUTTON,
                    // Against the rule, on purpose: the view's thread sleeps.
                    onclick: move |_| std::thread::sleep(std::time::Duration::from_secs(2)),
                    "Block this view for 2 s"
                }
            }
            div { style: "height: 8px; border-radius: 4px; background: var(--muted); overflow: hidden;",
                div { style: "height: 100%; width: {pct:.1}%; background: var(--clicker-count);" }
            }
            div { style: "font-size: 12px; color: var(--muted-foreground);", "{said}" }
        }
    }
}

/// A button's look, in the app's theme and the clicker's tokens.
const BUTTON: &str = "align-self: flex-start; padding: 6px 14px; border-radius: 8px; \
                      border: 1px solid var(--clicker-edge); background: var(--clicker-face); \
                      color: var(--foreground); font-size: 13px;";

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
            Primes {}
        }
    }
}

/// `LAB_CLICKER_PROBE`: its setting and the app's status, asked of the
/// host over its own connection (`guest::host`), checked there against
/// its capabilities. What it says on stderr is in its log (`EXT_LOGS`).
async fn probe() -> Result<serde_json::Value, String> {
    let step = host(HostRequest::Setting {
        key: "lab.clicker.step".to_owned(),
    })
    .await
    .map_err(|e| e.to_string())?;
    let status = host(HostRequest::Run {
        action: "APP_STATUS".to_owned(),
        args: serde_json::Value::Null,
        ticket: None,
    })
    .await
    .map_err(|e| e.to_string())?;
    let problems = status["problems"].as_array().map_or(0, Vec::len);
    // Its stderr is its log.
    eprintln!("probe: step {step}, {problems} problem(s)");
    Ok(serde_json::json!({ "step": step, "problems": problems, "pid": std::process::id() }))
}

fn main() -> Result<(), GuestError> {
    Guest::new()
        .view("counter", Counter)
        .action("LAB_CLICKER_CLICKS", |_| async {
            Ok(serde_json::json!(CLICKS.load(Ordering::Relaxed)))
        })
        .action("LAB_CLICKER_PROBE", |_| probe())
        .serve()
}
