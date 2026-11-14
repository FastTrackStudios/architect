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
//! | `list_documents {under?}` | the app's documents (`DocInfo`s), all or under a folder | `reads = ["documents"]` |
//! | `create_document {path, text?}` | a new document: its `DocInfo` | `writes = ["documents"]` |
//! | `edit_document {path, edits, name?}` | edits as one CRDT transaction, one undo step (`DocEdit`s): its text after | `writes = ["documents"]` |
//! | `rename_document {path, to}` / `move_document {path, folder}` | a new path (or folder) for it, its history kept: its `DocInfo` | `writes = ["documents"]` |
//! | `delete_document {path}` | deletes it | `writes = ["documents"]` |
//! | `watch_documents {action \| script, function?}` | its action (script) on every `doc.created/saved/renamed/deleted`: the hooks' keys | `reads = ["documents"]` |
//! | `editor_state` | the editor being typed in: `{editor, context, text, selection, primary, cursor, node, version}` (`crate::editor::EditorView`; `node` is the block at the cursor: a list item, a table cell and its table's cells, …) | `reads = ["editor"]` or `writes = ["editor"]` |
//! | `edit_editor {changes, selection?, name?, base?}` | its changes (byte offsets into the text it read) and selection as one transaction, one undo step named `name`, made as the extension (`architect_editor::ext::EditCommand`): the editor after | `writes = ["editor"]` |
//! | `decorate_editor {decorations}` | its marks, line and gutter decorations and widgets (`architect_editor::ext::DecorationSpec`; a widget mounts one of its own `[[embeds]]` or `[[views]]`) in the editor, in place of the ones it set before (`[]`: none) | `writes = ["editor"]` |
//! | `read_file {path}` / `write_file {path, text}` | a file: relative paths are in its own data folder | its own folder, or `capabilities.folders` |
//! | `setting {key}` / `set_setting {key, value}` | one of its own settings (`<id>.…`) | nothing |
//! | `data {key}` / `set_data {key, value?}` | its own data, kept for it | nothing |
//! | `log {text, level?}` | a line in its log (`EXT_LOGS`) | nothing |
//! | `service {service, method, args?}` | calls another extension's service: its provider's action `<PREFIX><METHOD>`, as this extension | `[requires] <service>`, and `capabilities.effect` |
//! | `point {point}` | the items of one of its own points (`[[points]]`) | owning it |
//! | `window {id, open, state?}` | opens or closes one of its own windows (`[[windows]]`) | nothing |
//! | `notify {text, kind?, title?}` | a notification (a toast) | `ui = ["notify"]` |
//! | `clipboard_read` / `clipboard_write {text}` | the clipboard's text, or sets it | `ui = ["clipboard"]` |
//! | `pick {kind, title?, filters?, folder?, name?}` | a file or folder picker, a save dialog, or a photo picker (`photo`, `photos`: images from the person's photos, copied for it): the path(s) picked, `null` when cancelled | `ui = ["pickers"]` |
//! | `prompt {kind, title, message?, default?, placeholder?, choices?}` | asks for text, a yes or no, or one of a list: the answer, `null` when cancelled | `ui = ["prompts"]` |
//! | `menu {items, x?, y?}` | a context menu of its own items at a point: the id chosen, `null` when dismissed | `ui = ["menus"]` |
//! | `decorate {target, decoration?}` | an icon or badge on a file-tree row or a tab (none: takes it off) | `ui = ["badges"]` |
//! | `subscribe {on \| every \| at, action \| script, function?}` | a hook made at run time: one of its actions (a Luau script) on an event or a schedule, as a manifest's `[[hooks]]`; its key | `capabilities.events` for an `app:<name>` event |
//! | `unsubscribe {key}` | takes a hook it made off: whether it had it | nothing |
//! | `events` | the events it can hear: the app's (`{name, title?}`), the app-defined ones it may hear, the hooks it made | nothing |
//! | `fetch {url, method?, headers?, body?, timeout_ms?, max_bytes?, stream?, binary?}` | an HTTP request ([`Fetch`]), redirects followed by the host: `{status, url, headers, body}` (`stream`: `{…, stream}`, its body read with `read_body`) | the URL's host, and every redirect's, in `capabilities.hosts` |
//! | `read_body {stream, max?}` / `close_body {stream}` | the next part of a streamed response's body (`{chunk, done}`), or stops reading it | its own stream |
//! | `secret {name}` / `ask_secret {name}` | whether the person has set one of its `[[secrets]]` (never its value), or asks them for it | declaring it |
//! | `ports {kind?}` | the MIDI ports the app has (`[{kind, name, direction}]`; OSC endpoints are not listed) | any `capabilities.ports` |
//! | `open_port {kind, name, listen?}` | opens a MIDI port by name, or an OSC endpoint (`host:port`, hearing on `listen`): `{port, kind, name}`; what it hears is a `port.message` event to it | `capabilities.ports` naming it (`midi:<name>`, `osc:<host>:<port>`, `osc:listen:<port>`) |
//! | `port_send {port, message}` / `port_receive {port, max?}` / `close_port {port}` | sends a message (`crate::ports`' JSON), takes what it heard and has not taken, closes it | its own port |
//! | `targets` | the kinds of target the app offers control surfaces (`[{id, title, min, max, step?, count, values}]`) | nothing |
//! | `surfaces` / `surface_bank {surface, bank}` | its control surfaces running (`[{id, port, bank, banks, error?}]`), or one of them on bank `bank` | its own |
//!
//! A request names a secret the host keeps (`crate::net::Secrets`) by
//! reference, in a header's value: `"Authorization": "Bearer
//! {{secret:api_key}}"`. The host puts the value in, only on a request to
//! one of that secret's hosts (`[[secrets]] hosts`), asking the person
//! for it the first time; the extension never reads it.
//!
//! `pick`, `prompt` and `menu` wait for the person: the call answers when
//! they do. A host with no way to ask (a headless one, a platform without
//! pickers) answers [`HostError::Unsupported`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capabilities::{
    BADGES, CLIPBOARD, DOCUMENTS, EDITOR, MENUS, NOTIFY, ORG, PICKERS, PROMPTS, SELECTION,
};
pub use crate::documents::{DocEdit, DocInfo};
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
    /// The app's documents (`crate::documents::Documents`), every one or
    /// those under folder `under`: `[DocInfo]`.
    ListDocuments {
        #[serde(default)]
        under: Option<String>,
    },
    /// A new document at `path` holding `text`: its [`DocInfo`].
    CreateDocument {
        path: String,
        #[serde(default)]
        text: String,
    },
    /// `edits`, in order, as one transaction: one step of the document's
    /// undo, named `name`, made as the extension. Its text after.
    EditDocument {
        path: String,
        edits: Vec<DocEdit>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Document `path` at `to`, its history kept: its [`DocInfo`].
    RenameDocument {
        path: String,
        to: String,
    },
    /// Document `path` into folder `folder` (`""`: the top), its name
    /// kept: its [`DocInfo`].
    MoveDocument {
        path: String,
        folder: String,
    },
    DeleteDocument {
        path: String,
    },
    /// Its action (a Luau script) on every document event —
    /// `doc.created`, `doc.saved`, `doc.renamed`, `doc.deleted` — as
    /// hooks made at run time: their keys.
    WatchDocuments {
        #[serde(default)]
        action: Option<String>,
        #[serde(default)]
        script: Option<String>,
        #[serde(default)]
        function: Option<String>,
    },
    /// The editor being typed in (`crate::editor`): its
    /// [`crate::editor::EditorView`].
    EditorState,
    /// `changes` and `selection` as one transaction of the editor being
    /// typed in, one undo step named `name`, made as the extension
    /// (`architect_editor::ext::EditCommand`): the editor after.
    EditEditor {
        #[serde(default)]
        changes: Vec<architect_editor::ext::TextChange>,
        #[serde(default)]
        selection: Option<Vec<architect_editor::ext::SelRange>>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        base: Option<u64>,
    },
    /// Its decorations in the editor being typed in, in place of the ones
    /// it set before.
    DecorateEditor {
        #[serde(default)]
        decorations: Vec<architect_editor::ext::DecorationSpec>,
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
    ClipboardWrite {
        text: String,
    },
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
    /// A hook made at run time (`crate::contrib::HookDecl`): one of its
    /// actions, or a Luau script, on an event or a schedule. Its answer is
    /// the hook's key.
    Subscribe(crate::contrib::HookDecl),
    /// Takes the hook it made with key `key` off.
    Unsubscribe {
        key: String,
    },
    /// The events it can hear, and the hooks it made.
    Events,
    /// Raises its own event `<its id>.<name>` with `detail`: what other
    /// extensions hear as `app:<its id>.<name>` when they may
    /// (`capabilities.events`). How many hooks it started.
    Raise {
        name: String,
        #[serde(default)]
        detail: Value,
    },
    /// An HTTP request, to a host it may reach.
    Fetch(Fetch),
    /// The next part, at most `max` bytes, of the body of a response
    /// fetched with `stream`: `{chunk, done}`.
    ReadBody {
        stream: String,
        #[serde(default)]
        max: Option<u64>,
    },
    /// Stops reading a streamed response: whether it was open.
    CloseBody {
        stream: String,
    },
    /// Whether the person has set its secret `name` (`[[secrets]]`):
    /// `{set, title}`, never the value.
    Secret {
        name: String,
    },
    /// Asks the person for its secret `name` (again, when it is set):
    /// whether they gave one.
    AskSecret {
        name: String,
    },
    /// The ports the app has, of kind `kind` (`midi`, `osc`) or every
    /// one: `[{kind, name, direction}]`.
    Ports {
        #[serde(default)]
        kind: Option<String>,
    },
    /// Opens `kind` port `name` (a MIDI port's name, an OSC endpoint's
    /// `host:port`), an OSC one hearing on local port `listen`: `{port,
    /// kind, name}`, `port` what the other port calls take. What it hears
    /// comes as `port.message` events to it, and waits for `port_receive`.
    OpenPort {
        kind: String,
        name: String,
        #[serde(default)]
        listen: Option<u16>,
    },
    /// Sends `message` (`{bytes}` or `{type, channel, number, value}` to
    /// a MIDI port, `{address, args}` to an OSC one) out of its port `port`.
    PortSend {
        port: String,
        message: Value,
    },
    /// What its port `port` heard and it has not taken, oldest first, at
    /// most `max`: `[message]`.
    PortReceive {
        port: String,
        #[serde(default)]
        max: Option<u32>,
    },
    /// Closes its port `port`: whether it was open.
    ClosePort {
        port: String,
    },
    /// The kinds of target the app offers control surfaces.
    Targets,
    /// Its control surfaces (`control.surfaces` items) as they run.
    Surfaces,
    /// Its control surface `surface` on bank `bank`.
    SurfaceBank {
        surface: String,
        bank: u32,
    },
}

/// An HTTP request an extension makes (`fetch`).
///
/// The host follows its redirects itself, each checked against
/// `capabilities.hosts`; it stops at a time limit and a size limit
/// (`crate::net` has the defaults and the most either may be).
///
/// Its answer: `{status, url, headers, body}` — `url` where it ended
/// after redirects, `headers` by lower-case name, `body` the text
/// (`binary`: base64). With `stream`, `{status, url, headers, stream}`:
/// the body is read a part at a time with `read_body`.
///
/// A header's value may name a secret, `{{secret:<name>}}`: the host puts
/// it in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fetch {
    pub url: String,
    /// `GET` when none.
    pub method: Option<String>,
    pub headers: BTreeMap<String, String>,
    /// What it sends, as text.
    pub body: Option<String>,
    /// The longest it waits for the response (and, `stream`, for each
    /// part of its body).
    pub timeout_ms: Option<u64>,
    /// The most of a body it takes: past it, the call fails.
    pub max_bytes: Option<u64>,
    /// Its body read with `read_body`, a part at a time.
    pub stream: bool,
    /// Its body as base64, not text.
    pub binary: bool,
}

/// The secrets a header's value names (`{{secret:<name>}}`), in order.
#[must_use]
pub fn secret_refs(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some((_, after)) = rest.split_once(SECRET_OPEN) {
        let Some((name, next)) = after.split_once("}}") else {
            break;
        };
        out.push(name.trim());
        rest = next;
    }
    out
}

/// What opens a secret's name in a header's value.
pub const SECRET_OPEN: &str = "{{secret:";

/// `value` with each secret it names replaced by `of(name)`.
///
/// # Errors
///
/// The first `of` refused.
pub fn with_secrets(
    value: &str,
    mut of: impl FnMut(&str) -> Result<String, HostError>,
) -> Result<String, HostError> {
    let mut out = String::new();
    let mut rest = value;
    while let Some((before, after)) = rest.split_once(SECRET_OPEN) {
        let Some((name, next)) = after.split_once("}}") else {
            break;
        };
        out.push_str(before);
        out.push_str(&of(name.trim())?);
        rest = next;
    }
    out.push_str(rest);
    Ok(out)
}

/// Whether an extension that may reach `may` may send a request to `url`
/// (an `http` or `https` URL whose host is in `capabilities.hosts`).
///
/// # Errors
///
/// The capability it lacks (`hosts`), and why.
pub fn may_reach_url(may: &Capabilities, url: &str) -> Result<url::Url, (String, String)> {
    let parsed = url::Url::parse(url)
        .map_err(|e| ("hosts".to_owned(), format!("{url} is not a URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err((
            "hosts".to_owned(),
            format!("{url}: only http and https are fetched"),
        ));
    }
    let host = parsed.host_str().unwrap_or_default();
    if may.may_reach_host(host, parsed.port_or_known_default()) {
        Ok(parsed)
    } else {
        Err((
            "hosts".to_owned(),
            format!("this extension may not reach {host} (capabilities.hosts)"),
        ))
    }
}

/// Whether `fetch` is one an extension of `manifest` that may reach `may`
/// may make: its URL's host one it may reach, and each secret it names
/// one it declares, sent only to that secret's hosts, in a header.
fn fetch_check(
    manifest: &Manifest,
    may: &Capabilities,
    fetch: &Fetch,
) -> Result<(), (String, String)> {
    let url = may_reach_url(may, &fetch.url)?;
    let host = url.host_str().unwrap_or_default();
    if !secret_refs(&fetch.url).is_empty()
        || fetch
            .body
            .as_deref()
            .is_some_and(|b| !secret_refs(b).is_empty())
    {
        return Err((
            "secrets".to_owned(),
            "a secret goes in a header's value, not the URL or the body".to_owned(),
        ));
    }
    for name in fetch.headers.values().flat_map(|v| secret_refs(v)) {
        let Some(secret) = manifest.secret(name) else {
            return Err((
                "secrets".to_owned(),
                format!("{name} is not one of its secrets ([[secrets]])"),
            ));
        };
        if !manifest.secret_reaches(secret, host, url.port_or_known_default()) {
            return Err((
                "secrets".to_owned(),
                format!("secret {name} is not sent to {host} (its [[secrets]] hosts)"),
            ));
        }
    }
    Ok(())
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
    /// One image from the person's photos: the path of a copy made for
    /// it. The system's photo picker (iOS: `PHPickerViewController`, out
    /// of the app's process, so no photo-library permission); on a
    /// platform without one, a file picker for images.
    Photo,
    /// Images from the person's photos: the paths of copies.
    Photos,
}

impl PickKind {
    /// Whether it answers a list (`open_many`, `photos`), not one path.
    #[must_use]
    pub const fn many(self) -> bool {
        matches!(self, Self::OpenMany | Self::Photos)
    }

    /// Whether it picks from the person's photos.
    #[must_use]
    pub const fn photos(self) -> bool {
        matches!(self, Self::Photo | Self::Photos)
    }
}

/// The image files a photo pick shows where the platform has no photo
/// picker of its own (a file picker in their place).
#[must_use]
pub fn image_filter() -> FileFilter {
    FileFilter {
        name: "Images".to_owned(),
        extensions: ["png", "jpg", "jpeg", "heic", "heif", "gif", "webp", "tiff"]
            .map(str::to_owned)
            .to_vec(),
    }
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
/// Its answer: a path (`open`, `folder`, `save`, `photo`), a list of
/// them (`open_many`, `photos`), or `null` when the person cancelled. A path picked is
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
    /// Text the screen hides as it is typed (a key, a password): the
    /// answer is a string. What the host asks for a secret it keeps.
    Secret,
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
            Self::ListDocuments { .. } => "list_documents",
            Self::CreateDocument { .. } => "create_document",
            Self::EditDocument { .. } => "edit_document",
            Self::RenameDocument { .. } => "rename_document",
            Self::MoveDocument { .. } => "move_document",
            Self::DeleteDocument { .. } => "delete_document",
            Self::WatchDocuments { .. } => "watch_documents",
            Self::EditorState => "editor_state",
            Self::EditEditor { .. } => "edit_editor",
            Self::DecorateEditor { .. } => "decorate_editor",
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
            Self::Subscribe(_) => "subscribe",
            Self::Unsubscribe { .. } => "unsubscribe",
            Self::Events => "events",
            Self::Raise { .. } => "raise",
            Self::Fetch(_) => "fetch",
            Self::ReadBody { .. } => "read_body",
            Self::CloseBody { .. } => "close_body",
            Self::Secret { .. } => "secret",
            Self::AskSecret { .. } => "ask_secret",
            Self::Ports { .. } => "ports",
            Self::OpenPort { .. } => "open_port",
            Self::PortSend { .. } => "port_send",
            Self::PortReceive { .. } => "port_receive",
            Self::ClosePort { .. } => "close_port",
            Self::Targets => "targets",
            Self::Surfaces => "surfaces",
            Self::SurfaceBank { .. } => "surface_bank",
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
        HostRequest::Open { .. }
        | HostRequest::ReadDocument { .. }
        | HostRequest::ListDocuments { .. }
        | HostRequest::WatchDocuments { .. } => ("reads", reads(DOCUMENTS)),
        HostRequest::CreateDocument { .. }
        | HostRequest::EditDocument { .. }
        | HostRequest::RenameDocument { .. }
        | HostRequest::MoveDocument { .. }
        | HostRequest::DeleteDocument { .. } => (
            "writes",
            if may.may_write(DOCUMENTS) {
                Ok(())
            } else {
                Err((
                    format!("writes.{DOCUMENTS}"),
                    format!(
                        "this extension may not change the app's {DOCUMENTS} (capabilities.writes)"
                    ),
                ))
            },
        ),
        HostRequest::EditorState => (
            "reads",
            if may.may_read(EDITOR) || may.may_write(EDITOR) {
                Ok(())
            } else {
                Err((
                    format!("reads.{EDITOR}"),
                    format!("this extension may not read the app's {EDITOR} (capabilities.reads)"),
                ))
            },
        ),
        HostRequest::EditEditor { .. } | HostRequest::DecorateEditor { .. } => (
            "writes",
            if may.may_write(EDITOR) {
                Ok(())
            } else {
                Err((
                    format!("writes.{EDITOR}"),
                    format!(
                        "this extension may not change the app's {EDITOR} (capabilities.writes)"
                    ),
                ))
            },
        ),
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
        HostRequest::Subscribe(hook) => (
            "events",
            match crate::events::app_event(&hook.on) {
                Some(name) if !may.may_hear(name) => Err((
                    format!("events.{name}"),
                    format!(
                        "this extension may not hear the app's {name} events (capabilities.events)"
                    ),
                )),
                // A document's events say which document: reading them is
                // reading the documents.
                _ if crate::events::is_doc_event(&hook.on) => reads(DOCUMENTS),
                _ => Ok(()),
            },
        ),
        // Its own events, under its id: no grant to raise them.
        HostRequest::Unsubscribe { .. } | HostRequest::Events | HostRequest::Raise { .. } => {
            ("events", Ok(()))
        }
        HostRequest::Fetch(fetch) => ("hosts", fetch_check(manifest, may, fetch)),
        // Its own streams: the host that keeps them checks whose they are.
        HostRequest::ReadBody { .. } | HostRequest::CloseBody { .. } => ("hosts", Ok(())),
        HostRequest::Secret { name } | HostRequest::AskSecret { name } => (
            "secrets",
            if manifest.secret(name).is_some() {
                Ok(())
            } else {
                Err((
                    "secrets".to_owned(),
                    format!("{name} is not one of its secrets ([[secrets]])"),
                ))
            },
        ),
        HostRequest::Ports { .. } => (
            "ports",
            if may.ports.is_empty() {
                Err((
                    "ports".to_owned(),
                    "this extension may open no MIDI port or OSC endpoint (capabilities.ports)"
                        .to_owned(),
                ))
            } else {
                Ok(())
            },
        ),
        HostRequest::OpenPort { kind, name, listen } => (
            "ports",
            if !matches!(kind.as_str(), "midi" | "osc") {
                Err((
                    "ports".to_owned(),
                    format!("{kind:?} is not a kind of port: midi or osc"),
                ))
            } else if may.may_open_port(kind, name, *listen) {
                Ok(())
            } else {
                let listening = listen.map_or_else(String::new, |p| format!(", hearing on {p},"));
                Err((
                    format!("ports.{kind}:{name}"),
                    format!(
                        "this extension may not open {kind} port {name}{listening} (capabilities.ports)"
                    ),
                ))
            },
        ),
        // Its own ports and surfaces: the host that keeps them checks
        // whose they are.
        HostRequest::PortSend { .. }
        | HostRequest::PortReceive { .. }
        | HostRequest::ClosePort { .. } => ("ports", Ok(())),
        HostRequest::Targets | HostRequest::Surfaces | HostRequest::SurfaceBank { .. } => {
            ("surfaces", Ok(()))
        }
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
        let r: HostRequest = serde_json::from_str(r#"{"op":"pick","kind":"photos"}"#).unwrap();
        let HostRequest::Pick(Pick { kind, .. }) = r else {
            panic!("{r:?}")
        };
        assert!(kind.many() && kind.photos());
        let r: HostRequest = serde_json::from_str(
            r#"{"op":"prompt","kind":"pick_one","title":"Which?","choices":[{"id":"a","label":"A"}]}"#,
        )
        .unwrap();
        let HostRequest::Prompt(Prompt { kind, choices, .. }) = &r else {
            panic!("{r:?}")
        };
        assert_eq!((*kind, choices.len()), (PromptKind::PickOne, 1));
        assert_eq!(r.op(), "prompt");
        let r: HostRequest =
            serde_json::from_str(r#"{"op":"subscribe","every":"5m","action":"X_TICK"}"#).unwrap();
        let HostRequest::Subscribe(hook) = &r else {
            panic!("{r:?}")
        };
        assert_eq!((hook.every.as_deref(), r.op()), (Some("5m"), "subscribe"));
        let r: HostRequest = serde_json::from_str(
            r#"{"op":"edit_document","path":"notes/a.md","edits":[{"insert":{"at":0,"text":"- "}},{"delete":{"at":5,"len":2}}]}"#,
        )
        .unwrap();
        let HostRequest::EditDocument { edits, .. } = &r else {
            panic!("{r:?}")
        };
        assert_eq!((edits.len(), r.op()), (2, "edit_document"));
        let r: HostRequest = serde_json::from_str(
            r#"{"op":"edit_editor","changes":[{"from":0,"to":0,"insert":"  "}],"name":"Indent"}"#,
        )
        .unwrap();
        let HostRequest::EditEditor { changes, name, .. } = &r else {
            panic!("{r:?}")
        };
        assert_eq!(
            (changes.len(), name.as_deref(), r.op()),
            (1, Some("Indent"), "edit_editor")
        );
        let r: HostRequest = serde_json::from_str(
            r#"{"op":"decorate_editor","decorations":[{"kind":"mark","from":0,"to":2,"class":"todo"}]}"#,
        )
        .unwrap();
        assert_eq!(r.op(), "decorate_editor");
        let r: HostRequest = serde_json::from_str(r#"{"op":"clipboard_read"}"#).unwrap();
        assert_eq!(r, HostRequest::ClipboardRead);
        let r: HostRequest = serde_json::from_str(
            r#"{"op":"decorate","target":{"kind":"file","id":"notes/a.md"},"decoration":{"badge":"3"}}"#,
        )
        .unwrap();
        assert_eq!(r.op(), "decorate");
    }
}
