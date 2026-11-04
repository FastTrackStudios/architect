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
//! | `notify {text, kind?, title?}` | a notification (a toast) | `ui = ["notify"]` |
//! | `clipboard_read` / `clipboard_write {text}` | the clipboard's text, or sets it | `ui = ["clipboard"]` |
//! | `pick {kind, title?, filters?, folder?, name?}` | a file or folder picker, or a save dialog: the path(s) picked, `null` when cancelled | `ui = ["pickers"]` |
//! | `prompt {kind, title, message?, default?, placeholder?, choices?}` | asks for text, a yes or no, or one of a list: the answer, `null` when cancelled | `ui = ["prompts"]` |
//! | `menu {items, x?, y?}` | a context menu of its own items at a point: the id chosen, `null` when dismissed | `ui = ["menus"]` |
//! | `decorate {target, decoration?}` | an icon or badge on a file-tree row or a tab (none: takes it off) | `ui = ["badges"]` |
//!
//! `pick`, `prompt` and `menu` wait for the person: the call answers when
//! they do. A host with no way to ask (a headless one, a platform without
//! pickers) answers [`HostError::Unsupported`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capabilities::{
    BADGES, CLIPBOARD, DOCUMENTS, MENUS, NOTIFY, ORG, PICKERS, PROMPTS, SELECTION,
};
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
    /// A notification: `kind` is `info` (none), `success`, `warning` or
    /// `error`.
    Notify {
        text: String,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        title: Option<String>,
    },
    /// The clipboard's text (`null` when it holds none).
    ClipboardRead,
    /// Puts `text` on the clipboard.
    ClipboardWrite { text: String },
    /// A picker or a save dialog: what the person picked.
    Pick(Pick),
    /// A question for the person: their answer.
    Prompt(Prompt),
    /// A context menu at a point: the item chosen.
    Menu(Menu),
    /// An icon or badge on a row or a tab, in place of the one it set
    /// there before (`decoration` none: takes it off).
    Decorate {
        target: Target,
        #[serde(default)]
        decoration: Option<Decoration>,
    },
}

/// What a picker picks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PickKind {
    /// One file to open: its path.
    #[default]
    Open,
    /// Files to open: their paths.
    OpenMany,
    /// A folder: its path.
    Folder,
    /// Where to save: the path (it need not exist).
    Save,
}

/// Files of one kind a picker shows (`Images`, `["png", "jpg"]`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFilter {
    pub name: String,
    /// Extensions, without the dot.
    pub extensions: Vec<String>,
}

/// A file or folder picker, or a save dialog.
///
/// Its answer: a path (`open`, `folder`, `save`), a list of them
/// (`open_many`), or `null` when the person cancelled. A path picked is
/// one the extension may read (`save`: write; `folder`: read and write
/// in) for the run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pick {
    pub kind: PickKind,
    pub title: Option<String>,
    pub filters: Vec<FileFilter>,
    /// The folder it starts in.
    pub folder: Option<String>,
    /// `save`: the file name it suggests.
    pub name: Option<String>,
}

/// What a prompt asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    /// Text: the answer is a string.
    #[default]
    Text,
    /// Yes or no: `true` or `false` (dismissed is no).
    Confirm,
    /// One of `choices`: its `id`.
    PickOne,
}

/// One of a `pick_one` prompt's choices, or a menu's items.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub id: String,
    pub label: String,
    /// A line under it.
    pub detail: Option<String>,
}

/// A question for the person. Its answer: the text (`text`), `true` or
/// `false` (`confirm`), the choice's id (`pick_one`); `null` when they
/// cancelled.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prompt {
    pub kind: PromptKind,
    pub title: String,
    pub message: Option<String>,
    /// `text`: what the field holds at first.
    pub default: Option<String>,
    pub placeholder: Option<String>,
    /// `pick_one`: what to pick from.
    pub choices: Vec<Choice>,
}

/// One item of a context menu.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenuItem {
    /// What the call answers when it is chosen.
    pub id: String,
    pub label: String,
    /// An icon's name (`architect_shell::named_icon`), or a character.
    pub icon: Option<String>,
    /// A key that does the same, shown beside it.
    pub shortcut: Option<String>,
    /// Shown, not chosen.
    pub disabled: bool,
    /// A line between items (its other fields unused).
    pub separator: bool,
}

/// A context menu of an extension's own items, at `x`, `y` in the
/// window's CSS pixels (none: the middle of the window). Its answer: the
/// id of the item chosen, or `null` when it was dismissed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub x: Option<f64>,
    pub y: Option<f64>,
}

// A menu's point is a number; its items and the rest compare exactly.
impl Eq for Menu {}

/// What a decoration is on: a file-tree row (`file`, its path) or a tab
/// (`tab`, its pane's kind or the path it shows). Others' kinds are the
/// app's to name.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Target {
    pub kind: String,
    pub id: String,
}

impl Target {
    /// A file-tree row: the file's path.
    #[must_use]
    pub fn file(path: impl Into<String>) -> Self {
        Self {
            kind: "file".to_owned(),
            id: path.into(),
        }
    }

    /// A tab: its pane's kind, or the path it shows.
    #[must_use]
    pub fn tab(id: impl Into<String>) -> Self {
        Self {
            kind: "tab".to_owned(),
            id: id.into(),
        }
    }
}

/// An icon or badge on a row or a tab: any of the four.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Decoration {
    /// An icon's name (`architect_shell::named_icon`), or a character
    /// (an emoji), before the label.
    pub icon: Option<String>,
    /// A short text after the label (`3`, `NEW`).
    pub badge: Option<String>,
    /// The badge's colour: a theme token (`primary`, `destructive`) or a
    /// CSS colour.
    pub color: Option<String>,
    /// What hovering it says.
    pub tooltip: Option<String>,
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
            Self::Notify { .. } => "notify",
            Self::ClipboardRead => "clipboard_read",
            Self::ClipboardWrite { .. } => "clipboard_write",
            Self::Pick(_) => "pick",
            Self::Prompt(_) => "prompt",
            Self::Menu(_) => "menu",
            Self::Decorate { .. } => "decorate",
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
    /// This app, or this platform, cannot do it (a headless host asked
    /// for a picker; a browser for a context menu).
    Unsupported { what: String },
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
            Self::Unsupported { what } => write!(f, "unsupported here: {what}"),
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
    let uses = |what: &str| {
        if may.may_use(what) {
            Ok(())
        } else {
            Err((
                format!("ui.{what}"),
                format!("this extension may not use the app's {what} (capabilities.ui)"),
            ))
        }
    };
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
        HostRequest::Notify { .. } => ("ui", uses(NOTIFY)),
        HostRequest::ClipboardRead | HostRequest::ClipboardWrite { .. } => ("ui", uses(CLIPBOARD)),
        HostRequest::Pick(_) => ("ui", uses(PICKERS)),
        HostRequest::Prompt(_) => ("ui", uses(PROMPTS)),
        HostRequest::Menu(_) => ("ui", uses(MENUS)),
        HostRequest::Decorate { .. } => ("ui", uses(BADGES)),
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

    #[test]
    fn the_interface_calls_are_plain_json_too() {
        use super::{Pick, PickKind, Prompt, PromptKind};
        let r: HostRequest =
            serde_json::from_str(r#"{"op":"pick","kind":"save","name":"a.md"}"#).unwrap();
        assert_eq!(
            r,
            HostRequest::Pick(Pick {
                kind: PickKind::Save,
                name: Some("a.md".to_owned()),
                ..Pick::default()
            })
        );
        let r: HostRequest = serde_json::from_str(
            r#"{"op":"prompt","kind":"pick_one","title":"Which?","choices":[{"id":"a","label":"A"}]}"#,
        )
        .unwrap();
        let HostRequest::Prompt(Prompt { kind, choices, .. }) = &r else {
            panic!("{r:?}")
        };
        assert_eq!((*kind, choices.len()), (PromptKind::PickOne, 1));
        assert_eq!(r.op(), "prompt");
        let r: HostRequest = serde_json::from_str(r#"{"op":"clipboard_read"}"#).unwrap();
        assert_eq!(r, HostRequest::ClipboardRead);
        let r: HostRequest = serde_json::from_str(
            r#"{"op":"decorate","target":{"kind":"file","id":"notes/a.md"},"decoration":{"badge":"3"}}"#,
        )
        .unwrap();
        assert_eq!(r.op(), "decorate");
    }
}
