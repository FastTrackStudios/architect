//! The Architect app's first run: with no vault yet, the Vault tab asks
//! for one. Open a folder the person has (an Obsidian vault reads as it
//! is), create a new one, or explore the demo vault. The choice is
//! remembered (`crate::vaults`), the vault opens in place, and the app
//! offers the Architect tour. Sign-in comes later (#188): its row is there,
//! disabled.

use std::path::PathBuf;
use std::sync::Arc;

use architect::action::ActionList;
use architect_shell as shell;
use dioxus::prelude::*;

/// The first run's card, filling the Vault tab. `on_open` is called once
/// the chosen vault is open.
#[component]
pub fn FirstRun(on_open: Callback<()>) -> Element {
    let list = use_context::<Arc<ActionList>>();
    let t = shell::use_shell().theme;
    let mut error = use_signal(|| None::<String>);
    let signed_in_ui = architect_accounts::ui::try_use_accounts();
    let choose = use_callback(move |dir: std::io::Result<PathBuf>| {
        let opened = dir.and_then(|dir| {
            crate::vaults::remember(&dir)?;
            crate::notes::register(&list);
            crate::notes::open_vault()
                .map(|_| dir.clone())
                .ok_or_else(|| std::io::Error::other(format!("{} is not a vault", dir.display())))
        });
        match opened {
            Ok(dir) => {
                tracing::info!(dir = %dir.display(), "app.first_run.vault_opened");
                on_open.call(());
            }
            Err(e) => error.set(Some(e.to_string())),
        }
    });
    let pick = move |_| {
        let picked = pick_folder();
        spawn(async move {
            if let Ok(Some(dir)) = picked.await {
                choose.call(Ok(dir));
            }
        });
    };
    let row = format!(
        "display: flex; flex-direction: column; gap: 2px; text-align: left; padding: 12px 14px; \
         border-radius: 10px; border: 1px solid {}; background: {}; color: {}; font-family: {}; cursor: pointer;",
        t.rule, t.desk, t.ink, t.font
    );
    let first = format!(
        "display: flex; flex-direction: column; gap: 2px; text-align: left; padding: 12px 14px; \
         border-radius: 10px; border: 1px solid {}; background: {}; color: {}; font-family: {}; cursor: pointer;",
        t.accent, t.accent, t.accent_ink, t.font
    );
    let label = "font-size: 14px; font-weight: 650;";
    let about = format!("font-size: 12.5px; color: {};", t.ink_2);
    let about_first = "font-size: 12.5px; opacity: 0.85;";
    rsx! {
        div {
            "data-testid": "first-run",
            style: "width: 100%; height: 100%; display: flex; align-items: center; justify-content: center; \
                    background: {t.desk}; font-family: {t.font}; color: {t.ink};",
            div {
                style: "width: 440px; max-width: 92%; display: flex; flex-direction: column; gap: 10px; padding: 28px; \
                        border-radius: 16px; border: 1px solid {t.rule}; background: {t.sheet}; box-shadow: {t.shadow_float};",
                div { style: "display: flex; align-items: center; gap: 10px; margin-bottom: 4px;",
                    span { style: "width: 32px; height: 32px; border-radius: 8px; background: {t.ink}; color: {t.desk}; \
                                   display: flex; align-items: center; justify-content: center; font-size: 17px; font-weight: 800;",
                        "A"
                    }
                    span { style: "font-size: 22px; font-weight: 750; letter-spacing: -0.03em;", "Welcome to Architect" }
                }
                p { style: "margin: 0 0 8px 0; font-size: 13.5px; line-height: 1.45; color: {t.ink_2};",
                    "Architect keeps your notes in a vault: a folder of markdown files on this device. Choose one to start."
                }
                if can_pick() {
                    button { "data-testid": "first-run-open", style: "{first}", onclick: pick,
                        span { style: label, "Open a folder…" }
                        span { style: about_first, "A folder of notes you already have. An Obsidian vault opens as it is." }
                    }
                }
                button { "data-testid": "first-run-create", style: "{row}",
                    onclick: move |_| choose.call(crate::vaults::create()),
                    span { style: label, "Create a new vault" }
                    span { style: "{about}", "An empty vault in Documents › Architect." }
                }
                button { "data-testid": "first-run-demo", style: "{row}",
                    onclick: move |_| choose.call(crate::vaults::demo()),
                    span { style: label, "Explore the demo vault" }
                    span { style: "{about}", "Made-up notes, projects, bases and a journal to try things on." }
                }
                div { style: "height: 1px; background: {t.rule}; margin: 6px 0;" }
                // Optional: the sheet, over the welcome (#188).
                if let Some(accounts) = signed_in_ui {
                    button { "data-testid": "first-run-sign-in", style: "{row}",
                        onclick: move |_| accounts.accounts().sign_in(),
                        span { style: label, "Sign in" }
                        span { style: "{about}", "Optional: your account, its devices and, soon, sync." }
                    }
                } else {
                    div { "data-testid": "first-run-sign-in", style: "{row} opacity: 0.5; cursor: default;",
                        span { style: label, "Sign in" }
                        span { style: "{about}", "Coming soon: your account, organizations and sync." }
                    }
                }
                if let Some(e) = error() {
                    p { style: "margin: 4px 0 0 0; font-size: 12.5px; color: {t.danger};", "{e}" }
                }
                p { style: "margin: 6px 0 0 0; font-size: 12px; color: {t.ink_3};",
                    "After that, Architect offers its tour. ⌘K › Take the tour plays it any time."
                }
            }
        }
    }
}

/// Whether this platform has a folder picker (a desk does).
const fn can_pick() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))
}

/// The system's folder picker, on a thread of its own: the folder chosen,
/// or `None` when it was cancelled.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn pick_folder() -> futures_channel::oneshot::Receiver<Option<PathBuf>> {
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .set_title("Open a folder as a vault")
            .pick_folder();
        let _ = tx.send(picked);
    });
    rx
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn pick_folder() -> futures_channel::oneshot::Receiver<Option<PathBuf>> {
    let (tx, rx) = futures_channel::oneshot::channel();
    let _ = tx.send(None);
    rx
}
