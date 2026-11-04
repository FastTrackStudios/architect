//! The host service's calls, as data (#178): what an extension asks the
//! app for, the same over every way it runs.
//!
//! A [`HostRequest`] is one call; its answer is a [`HostAnswer`]. Both
//! are plain JSON, so each way an extension runs carries them as they
//! are — a process over vox (`ExtensionHost::call`, `guest::host`), a
//! Luau script through its `host` global, an in-process extension through
//! its `linked::Handle`, and the WASM runner (#167) through one imported
//! function taking and giving JSON text. The host side, which checks each
//! call against what the extension may reach, is `crate::hostapi`
//! (feature `host`), and the browser's `crate::web` `WebHost` (feature
//! `web`); both decide with [`check`].
//!
//! | call | what it asks | needs |
//! |---|---|---|
//! | `run {action, args?}` | runs an app action, as the extension (for whoever it acts for) | its own namespace, or `capabilities.actions` and `.effect` |
//! | `org` | the current organization | `reads = ["org"]` |
//! | `selection` | what is selected and focused | `reads = ["selection"]` |
//! | `open {path}` | opens a document or file in the app | `reads = ["documents"]` |
//! | `read_document {path}` | a document's text | `reads = ["documents"]` |
//! | `read_file {path}` / `write_file {path, text}` | a file: relative paths are in its own data folder | its own folder, or `capabilities.folders` |
//! | `setting {key}` / `set_setting {key, value}` | one of its own settings (`<id>.…`) | nothing |
//! | `data {key}` / `set_data {key, value?}` | its own data, kept for it | nothing |
//! | `log {text, level?}` | a line in its log (`EXT_LOGS`) | nothing |
//! | `service {service, method, args?}` | calls another extension's service: its provider's action `<PREFIX><METHOD>`, as this extension | `[requires] <service>`, and `capabilities.effect` |
//! | `point {point}` | the items of one of its own points (`[[points]]`) | owning it |
//! | `window {id, open, state?}` | opens or closes one of its own windows (`[[windows]]`) | nothing |

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capabilities::{DOCUMENTS, ORG, SELECTION};
use crate::manifest::{Capabilities, Manifest, Reach};

/// One call an extension makes to its host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostRequest {
    Run {
        action: String,
        #[serde(default)]
        args: Value,
        /// The ticket a person approved, when the first run answered
        /// [`HostError::ConfirmationRequired`].
        #[serde(default)]
        ticket: Option<String>,
    },
    Org,
    Selection,
    Open {
        path: String,
    },
    ReadDocument {
        path: String,
    },
    ReadFile {
        path: String,
    },
    WriteFile {
        path: String,
        text: String,
    },
    Setting {
        key: String,
    },
    SetSetting {
        key: String,
        value: Value,
    },
    Data {
        key: String,
    },
    SetData {
        key: String,
        #[serde(default)]
        value: Option<Value>,
    },
    Log {
        text: String,
        #[serde(default)]
        level: Option<String>,
    },
    /// Calls method `method` of service `service` (one it `[requires]`),
    /// answered by its provider's action `<PREFIX><METHOD>`.
    Service {
        service: String,
        method: String,
        #[serde(default)]
        args: Value,
    },
    /// The items of point `point`, one it declares: `[{from, value}]`.
    Point {
        point: String,
    },
    /// Opens (`open`, with `state`) or closes one of its own windows.
    Window {
        id: String,
        #[serde(default = "yes")]
        open: bool,
        #[serde(default)]
        state: Value,
    },
}

const fn yes() -> bool {
    true
}

impl HostRequest {
    /// Its name (`run`, `read_file`): the `op` it is sent as.
    #[must_use]
    pub const fn op(&self) -> &'static str {
        match self {
            Self::Run { .. } => "run",
            Self::Org => "org",
            Self::Selection => "selection",
            Self::Open { .. } => "open",
            Self::ReadDocument { .. } => "read_document",
            Self::ReadFile { .. } => "read_file",
            Self::WriteFile { .. } => "write_file",
            Self::Setting { .. } => "setting",
            Self::SetSetting { .. } => "set_setting",
            Self::Data { .. } => "data",
            Self::SetData { .. } => "set_data",
            Self::Log { .. } => "log",
            Self::Service { .. } => "service",
            Self::Point { .. } => "point",
            Self::Window { .. } => "window",
        }
    }
}

/// Why the host did not do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostError {
    /// Outside what it may reach: the capability it would need
    /// (`actions`, `effect`, `reads.selection`, `folders`, `settings`)
    /// and why, in words. Granting it is the person's
    /// (`EXT_GRANT`, the extensions page).
    Refused { capability: String, why: String },
    /// A person must say yes to this exact call first (an extension acting
    /// for an agent ran something destructive): run it again with the
    /// ticket once they have.
    ConfirmationRequired { action: String, ticket: String },
    /// The extension is not loaded (unloaded, or replaced) any more.
    NotLoaded { id: String },
    /// Nothing of that name (an action, a document, a file).
    NotFound { what: String },
    /// It was allowed and did not work.
    Failed { message: String },
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused { capability, why } => write!(f, "refused ({capability}): {why}"),
            Self::ConfirmationRequired { action, ticket } => {
                write!(
                    f,
                    "{action}: a person must confirm it first (ticket {ticket})"
                )
            }
            Self::NotLoaded { id } => write!(f, "{id} is not loaded"),
            Self::NotFound { what } => write!(f, "no {what}"),
            Self::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for HostError {}

/// A call's answer: `{"ok": value}` or `{"error": {kind, …}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostAnswer {
    Ok(Value),
    Error(HostError),
}

impl HostAnswer {
    /// The answer as a `Result`.
    ///
    /// # Errors
    ///
    /// The host refused or failed.
    pub fn into_result(self) -> Result<Value, HostError> {
        match self {
            Self::Ok(v) => Ok(v),
            Self::Error(e) => Err(e),
        }
    }

    /// As JSON text (what crosses a wire).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|e| format!(r#"{{"error":{{"kind":"failed","message":"{e}"}}}}"#))
    }

    /// From JSON text.
    #[must_use]
    pub fn from_json(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_else(|e| {
            Self::Error(HostError::Failed {
                message: format!("the host's answer is not one: {e}"),
            })
        })
    }
}

impl From<Result<Value, HostError>> for HostAnswer {
    fn from(r: Result<Value, HostError>) -> Self {
        match r {
            Ok(v) => Self::Ok(v),
            Err(e) => Self::Error(e),
        }
    }
}

/// Whether an extension that may reach `may` may make `request`.
///
/// `may` is its manifest's capabilities as a person granted them; the
/// answer is the capability it asks for, and the one it lacks and why when
/// it may not. `effect` is what an app action `run` names changes (the
/// list knows; the caller looks it up).
///
/// The one decision every host makes: `hostapi::ExtensionApi` (natively)
/// and `web::WebHost` (in a browser). A file's path is checked by the host
/// that has files (its own folder, `capabilities.folders`).
///
/// # Errors
///
/// The capability it lacks, and why.
pub fn check(
    manifest: &Manifest,
    may: &Capabilities,
    request: &HostRequest,
    effect: Reach,
) -> (&'static str, Result<(), (String, String)>) {
    let reads = |what: &str| {
        if may.may_read(what) {
            Ok(())
        } else {
            Err((
                format!("reads.{what}"),
                format!("this extension may not read the app's {what} (capabilities.reads)"),
            ))
        }
    };
    match request {
        HostRequest::Run { action, .. } => (
            "actions",
            may.may_run(&manifest.action_prefix(), action, effect)
                .map_err(|(c, why)| (c.to_owned(), why)),
        ),
        HostRequest::Org => ("reads", reads(ORG)),
        HostRequest::Selection => ("reads", reads(SELECTION)),
        HostRequest::Open { .. } | HostRequest::ReadDocument { .. } => ("reads", reads(DOCUMENTS)),
        HostRequest::Setting { key } | HostRequest::SetSetting { key, .. } => {
            let check = if key.starts_with(&format!("{}.", manifest.id)) {
                Ok(())
            } else {
                Err((
                    "settings".to_owned(),
                    format!(
                        "{key} is not one of its settings: an extension reads and writes only its own ({}.…)",
                        manifest.id
                    ),
                ))
            };
            ("settings", check)
        }
        HostRequest::Data { .. } | HostRequest::SetData { .. } => ("data", Ok(())),
        HostRequest::ReadFile { .. } | HostRequest::WriteFile { .. } => ("files", Ok(())),
        HostRequest::Log { .. } => ("log", Ok(())),
        // Checked by the host that answers them: a service against its
        // [requires] and the method's effect, a point and a window as its own.
        HostRequest::Service { .. } => ("requires", Ok(())),
        HostRequest::Point { .. } => ("points", Ok(())),
        HostRequest::Window { .. } => ("windows", Ok(())),
    }
}

#[cfg(test)]
mod tests {
    use super::{HostAnswer, HostError, HostRequest};

    #[test]
    fn calls_and_answers_are_plain_json() {
        let r: HostRequest = serde_json::from_str(r#"{"op":"run","action":"APP_STATUS"}"#).unwrap();
        assert_eq!(r.op(), "run");
        let r: HostRequest = serde_json::from_str(r#"{"op":"selection"}"#).unwrap();
        assert_eq!(r, HostRequest::Selection);
        let refused = HostAnswer::Error(HostError::Refused {
            capability: "actions".to_owned(),
            why: "no".to_owned(),
        });
        let text = refused.to_json();
        assert!(text.contains(r#""kind":"refused""#), "{text}");
        assert_eq!(HostAnswer::from_json(&text), refused);
        assert_eq!(
            HostAnswer::from_json(r#"{"ok":3}"#).into_result(),
            Ok(serde_json::json!(3))
        );
    }
}
