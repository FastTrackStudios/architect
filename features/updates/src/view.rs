//! The settings section (`[[sections]] view = "settings"`): the running
//! version, the channel, the last check and Check now; the channel and
//! automatic checks; Install or Restart when there is one.

use std::sync::atomic::Ordering;
use std::time::Duration;

use architect_extensions::hostcall::HostRequest;
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::model::{Channel, Plan, State, percent};
use crate::{REVISION, settings, shared};

const MUTED: &str = "color: var(--muted-foreground);";
const CARD: &str = "display: flex; flex-direction: column; gap: 10px; padding: 14px 16px; \
                    border-radius: 10px; border: 1px solid var(--border); background: var(--card);";

fn button(primary: bool) -> String {
    format!(
        "padding: 4px 12px; border-radius: 6px; font-size: 13px; cursor: pointer; border: 1px solid var(--border); \
         background: {}; color: {};",
        if primary {
            "var(--primary)"
        } else {
            "transparent"
        },
        if primary {
            "var(--primary-foreground)"
        } else {
            "var(--foreground)"
        }
    )
}

/// Redraws when the updater's state moves.
fn use_revision() -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(Duration::from_millis(250)).await;
            let now = REVISION.load(Ordering::Relaxed);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

fn describe(state: &State) -> String {
    match state {
        State::Idle => "Not checked yet".to_owned(),
        State::DevelopmentBuild => crate::model::ReportOnly::DevelopmentBuild.reason(),
        State::Checking => "Checking…".to_owned(),
        State::UpToDate { newest: Some(v) } => format!("Up to date (newest release: {v})"),
        State::UpToDate { newest: None } => "Up to date (no release yet)".to_owned(),
        State::Available {
            version,
            signed: false,
            ..
        } => format!("{version} is out, but it is not signed, so it is not installed"),
        State::Available {
            version,
            plan: Plan::ReportOnly(why),
            ..
        } => format!("{version} is out: {}", why.reason()),
        State::Available { version, .. } => format!("{version} is available"),
        State::Downloading {
            version,
            received,
            total,
        } => percent(*received, *total).map_or_else(
            || format!("Downloading {version}…"),
            |p| format!("Downloading {version}… {p}%"),
        ),
        State::Ready {
            version,
            plan: Plan::InPlace { .. },
        } => format!("{version} is installed: restart to finish"),
        State::Ready { version, .. } => {
            format!("{version} is installed: quit and open Architect again to finish")
        }
        State::Failed { reason } => format!("The last try failed: {reason}"),
    }
}

/// `text` with its first letter upper-case.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

async fn set_setting(key: &str, value: Value) {
    if let Some(h) = crate::handle() {
        let _ = h
            .host(HostRequest::SetSetting {
                key: key.to_owned(),
                value,
            })
            .await;
    }
}

#[allow(non_snake_case)]
pub fn Settings() -> Element {
    let _ = use_revision();
    let mut auto = use_signal(|| true);
    use_future(move || async move {
        if let Some(on) = crate::setting(settings::AUTO).await.as_bool() {
            auto.set(on);
        }
        let _ = crate::channel().await;
        REVISION.fetch_add(1, Ordering::Relaxed);
    });
    let (state, channel, last) = {
        let s = shared();
        (s.state.clone(), s.channel, s.last_check.clone())
    };
    let version = crate::release_version().map_or_else(
        || "Development build".to_owned(),
        |v| format!("Architect {v}"),
    );
    let last = last.unwrap_or_else(|| "not this session".to_owned());
    let line = sentence(&describe(&state));
    let installable = matches!(
        &state,
        State::Available { signed: true, plan, .. } if plan.installs()
    );
    let has_page = matches!(&state, State::Available { .. });
    let ready = matches!(&state, State::Ready { .. });
    let busy = matches!(&state, State::Checking | State::Downloading { .. });
    let dev = matches!(&state, State::DevelopmentBuild);
    let run =
        |f: fn(Value) -> std::pin::Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>| {
            move |_| {
                spawn(async move {
                    let _ = f(Value::Null).await;
                });
            }
        };
    let pick = move |c: Channel| {
        move |_| {
            spawn(async move {
                set_setting(settings::CHANNEL, json!(c.as_str())).await;
                shared().channel = c;
                REVISION.fetch_add(1, Ordering::Relaxed);
            });
        }
    };
    rsx! {
        div { style: "max-width: 760px; display: flex; flex-direction: column; gap: 18px;",
            div { style: CARD, "data-updates": "status",
                div { style: "display: flex; justify-content: space-between; align-items: baseline; gap: 12px;",
                    div { style: "font-weight: 600;", "{version}" }
                    div { style: "{MUTED} font-size: 12px;", "Channel {channel.as_str()} · last checked {last}" }
                }
                div { style: "font-size: 13px;", "{line}" }
                div { style: "display: flex; gap: 8px; flex-wrap: wrap;",
                    if !dev {
                        button { style: button(!installable && !ready), disabled: busy,
                            onclick: run(|a| Box::pin(crate::check(a))), "Check now" }
                    }
                    if installable {
                        button { style: button(true), onclick: run(|a| Box::pin(crate::install(a))), "Install" }
                    }
                    if ready {
                        button { style: button(true), onclick: run(|a| Box::pin(crate::restart(a))), "Restart to finish" }
                    }
                    if has_page {
                        button { style: button(false), onclick: run(|a| Box::pin(crate::page(a))), "Release page" }
                    }
                }
            }
            div { style: CARD,
                div { style: "display: flex; justify-content: space-between; align-items: center; gap: 12px;",
                    div {
                        div { "Update channel" }
                        div { style: "{MUTED} font-size: 12px;",
                            if channel == Channel::Nightly {
                                "{Channel::Nightly.available().err().unwrap_or_default()}"
                            } else {
                                "Signed releases only."
                            }
                        }
                    }
                    div { style: "display: flex; gap: 6px;",
                        button { style: button(channel == Channel::Stable), onclick: pick(Channel::Stable), "Stable" }
                        button { style: button(channel == Channel::Nightly), onclick: pick(Channel::Nightly), "Nightly" }
                    }
                }
                div { style: "display: flex; justify-content: space-between; align-items: center; gap: 12px;",
                    div {
                        div { "Check automatically" }
                        div { style: "{MUTED} font-size: 12px;", "At start and once a day. Updates download only when you choose Install." }
                    }
                    button {
                        style: button(auto()),
                        onclick: move |_| {
                            let on = !auto();
                            auto.set(on);
                            spawn(async move { set_setting(settings::AUTO, json!(on)).await });
                        },
                        if auto() { "On" } else { "Off" }
                    }
                }
            }
        }
    }
}
