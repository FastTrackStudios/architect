//! `CAPTURE_*` and `INBOX_*` on an Action List (feature `actions`): MCP,
//! ⌘K, the menus, the CLI and the composer all capture and process
//! through them.
//!
//! | action | what it does |
//! |---|---|
//! | `CAPTURE_OPEN` | opens the composer (`window: true`: the floating capture window) |
//! | `CAPTURE_ADD` | captures `text` into the inbox, or straight into a `target` |
//! | `CAPTURE_PARSE` | reads `text` as quick add: its title, properties and the target it suggests |
//! | `CAPTURE_TARGETS` | what a capture can become: the app's note and every extension's `[[capture_targets]]` |
//! | `INBOX_OPEN` | opens the inbox deck |
//! | `INBOX_LIST` | the captures waiting (`all`: those put off too) |
//! | `INBOX_PROCESS` | turns capture `path` into `target`'s thing |
//! | `INBOX_FILE` | files capture `path` as a plain note in `folder` |
//! | `INBOX_DEFER` | puts capture `path` off until `until` |
//! | `INBOX_DISCARD` | deletes capture `path` |
//!
//! It owns no storage: the inbox is notes, read and written through the
//! vault's actions (`VAULT_*`), and a target is an extension's action
//! (`crate::target`). Each runs those as whoever ran it, so an agent's
//! capture is checked as its own vault writes would be. Every change is
//! a `Changed { topic: "inbox" }` on the list.

use std::sync::{Arc, Weak};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use chrono::{Duration, Local, NaiveDate};
use facet::Facet;
use serde_json::{Map, Value, json};

use crate::inbox::{self, Item};
use crate::parse::{self, Draft};
use crate::target::{self, Capture, Target};

/// The actions' ids.
pub mod ids {
    pub const OPEN: &str = "CAPTURE_OPEN";
    pub const ADD: &str = "CAPTURE_ADD";
    pub const PARSE: &str = "CAPTURE_PARSE";
    pub const TARGETS: &str = "CAPTURE_TARGETS";
    pub const INBOX_OPEN: &str = "INBOX_OPEN";
    pub const INBOX_LIST: &str = "INBOX_LIST";
    pub const INBOX_PROCESS: &str = "INBOX_PROCESS";
    pub const INBOX_FILE: &str = "INBOX_FILE";
    pub const INBOX_DEFER: &str = "INBOX_DEFER";
    pub const INBOX_DISCARD: &str = "INBOX_DISCARD";
    /// Every id, in menu order.
    pub const ALL: [&str; 10] = [
        OPEN,
        ADD,
        PARSE,
        TARGETS,
        INBOX_OPEN,
        INBOX_LIST,
        INBOX_PROCESS,
        INBOX_FILE,
        INBOX_DEFER,
        INBOX_DISCARD,
    ];
}

/// The host's registry of extensions' targets (`[[capture_targets]]`).
pub const REGISTRY: &str = "EXT_CAPTURE_TARGETS";

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Capture",
        method_name: id,
        display_name: name,
        description,
        category: "Capture",
        group: "Inbox",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static OPEN: ActionMeta = meta(
    ids::OPEN,
    "Capture…",
    "Opens the composer: a line read as you type (`[[Project]]`, `tomorrow`, `!high`, `#tag`, \
     `@context`, `every week` become chips), a target to make it, ⌘↩ to capture. `text` fills \
     it, `target` picks what it becomes; `window` true opens the floating capture window over \
     every app instead (the global shortcut's).",
);
static ADD: ActionMeta = meta(
    ids::ADD,
    "Capture",
    "Captures `text`: its first line read as quick add (`[[Project]]`, `#tag`, `@context`, \
     `!high`, `today` / `tomorrow` / a weekday / `next fri` / YYYY-MM-DD, `every week`; the rest \
     its title), the lines after it the body. `properties` (a JSON object) are set by hand and \
     beat the text; `null` takes one away. It lands in the inbox as a note, unless `target` (a \
     CAPTURE_TARGETS id: `note`, `task`, …) makes it that at once. `source` says where it came \
     from (default `mcp`). Answers the note written and its properties.",
);
static PARSE: ActionMeta = meta(
    ids::PARSE,
    "Read a capture",
    "Reads `text` as quick add without writing anything: its title, the properties its tokens \
     give (extensions' tokens too) and the target it suggests (`task` when it has a project, a \
     context, a date, a repeat or a priority, else `note`).",
);
static TARGETS: ActionMeta = meta(
    ids::TARGETS,
    "Capture targets",
    "What a capture can become: the app's plain note, then every loaded extension's \
     `[[capture_targets]]` (id, name, icon, the chips it offers, the quick-add tokens it adds, \
     the action that makes one, its extension).",
);
static INBOX_OPEN: ActionMeta = meta(
    ids::INBOX_OPEN,
    "Process inbox",
    "Opens the inbox deck: each capture once, to file as a note, make into a target's thing, \
     put off, or discard.",
);
static INBOX_LIST: ActionMeta = meta(
    ids::INBOX_LIST,
    "Inbox",
    "The captures waiting in the inbox, oldest first: path, title, body, source, when, and \
     their properties. Those put off until a later day are left out unless `all`.",
);
static INBOX_PROCESS: ActionMeta = meta(
    ids::INBOX_PROCESS,
    "Process a capture",
    "Turns capture `path` (from INBOX_LIST) into `target`'s thing (a CAPTURE_TARGETS id: `task`, \
     …; `note` files it as INBOX_FILE does), with its title (or `title`), body and properties, \
     `properties` set by hand over them. The capture leaves the inbox. Answers what was made.",
);
static INBOX_FILE: ActionMeta = meta(
    ids::INBOX_FILE,
    "File a capture",
    "Files capture `path` as a plain note: moved to `folder` (the vault's top when unset), its \
     properties kept and the inbox's own (type, status, source, created) taken off.",
);
static INBOX_DEFER: ActionMeta = meta(
    ids::INBOX_DEFER,
    "Put a capture off",
    "Puts capture `path` off until `until` (YYYY-MM-DD, or `tomorrow`, a weekday, `next mon`; \
     tomorrow when unset): it stays out of the inbox until that day.",
);
static INBOX_DISCARD: ActionMeta = meta(
    ids::INBOX_DISCARD,
    "Discard a capture",
    "Deletes capture `path` from the inbox and the vault.",
);

/// What the composer is asked to show (`CAPTURE_OPEN`, `INBOX_OPEN`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// The composer, in the window, filled with `text`.
    Composer {
        text: String,
        target: Option<String>,
    },
    /// The floating capture window.
    Window { text: String },
    /// The inbox deck.
    Inbox,
}

/// Shows what [`Open`] asks for: the app's window side (called on any
/// thread: send it on).
pub type Opener = Arc<dyn Fn(Open) + Send + Sync>;

/// Where captures land and are filed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The inbox's folder ([`inbox::FOLDER`]).
    pub folder: String,
    /// Where a capture filed with no folder goes (the vault's top).
    pub notes: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            folder: inbox::FOLDER.to_owned(),
            notes: String::new(),
        }
    }
}

/// Captures and processes over an Action List's vault and targets: what
/// the actions run, and what the composer and the deck call.
#[derive(Clone)]
pub struct Capturer {
    list: Weak<ActionList>,
    config: Config,
}

impl PartialEq for Capturer {
    fn eq(&self, other: &Self) -> bool {
        self.list.ptr_eq(&other.list) && self.config == other.config
    }
}

/// Today, where the person is.
#[must_use]
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// `n` days after today (today again past the calendar's end).
#[must_use]
pub fn day_after(n: i64) -> NaiveDate {
    let today = today();
    today.checked_add_signed(Duration::days(n)).unwrap_or(today)
}

/// What `CAPTURE_ADD` is given (and the composer captures).
#[derive(Clone, Debug, Default, Facet)]
pub struct AddArgs {
    pub text: String,
    /// Set by hand: beat the text's.
    #[facet(default)]
    pub properties: Option<facet_value::Value>,
    /// A target's id: made at once, not left in the inbox.
    #[facet(default)]
    pub target: Option<String>,
    /// `ui`, `window`, `mcp` (the default), `cli`.
    #[facet(default)]
    pub source: Option<String>,
}

/// `CAPTURE_OPEN`'s arguments.
#[derive(Clone, Debug, Default, Facet)]
pub struct OpenArgs {
    #[facet(default)]
    pub text: Option<String>,
    #[facet(default)]
    pub target: Option<String>,
    #[facet(default)]
    pub window: Option<bool>,
}

/// `CAPTURE_PARSE`'s arguments.
#[derive(Clone, Debug, Facet)]
pub struct TextArgs {
    pub text: String,
}

/// `INBOX_LIST`'s arguments.
#[derive(Clone, Debug, Default, Facet)]
pub struct ListArgs {
    #[facet(default)]
    pub all: Option<bool>,
}

/// `INBOX_PROCESS`'s arguments.
#[derive(Clone, Debug, Facet)]
pub struct ProcessArgs {
    pub path: String,
    pub target: String,
    #[facet(default)]
    pub title: Option<String>,
    #[facet(default)]
    pub properties: Option<facet_value::Value>,
}

/// `INBOX_FILE`'s arguments.
#[derive(Clone, Debug, Facet)]
pub struct FileArgs {
    pub path: String,
    #[facet(default)]
    pub folder: Option<String>,
}

/// `INBOX_DEFER`'s arguments.
#[derive(Clone, Debug, Facet)]
pub struct DeferArgs {
    pub path: String,
    #[facet(default)]
    pub until: Option<String>,
}

/// `INBOX_DISCARD`'s arguments.
#[derive(Clone, Debug, Facet)]
pub struct PathArgs {
    pub path: String,
}

/// A facet value as JSON's (an object's properties; anything else none).
fn props_of(v: Option<&facet_value::Value>) -> Map<String, Value> {
    v.and_then(|v| facet_json::to_string(v).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn raw(v: &Value) -> RawJson {
    RawJson::from(v.to_string())
}

impl Capturer {
    /// Over `list` (held weakly: the list owns the actions that hold it).
    #[must_use]
    pub fn new(list: &Arc<ActionList>, config: Config) -> Self {
        Self {
            list: Arc::downgrade(list),
            config,
        }
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Runs `id` with `args` as `caller`: its answer (`null` for none).
    async fn run(&self, caller: &ActionCaller, id: &str, args: Value) -> Result<Value, String> {
        let list = self.list.upgrade().ok_or("the app is closing")?;
        let out = list
            .run_json_async_with(caller, id, args.to_string().as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        if out.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&out).map_err(|e| format!("{id} answered {e}"))
    }

    fn changed(&self, detail: &str) {
        if let Some(list) = self.list.upgrade() {
            list.changed("inbox", detail);
        }
    }

    /// What a capture can become: the note, then extensions' targets.
    pub async fn targets(&self, caller: &ActionCaller) -> Vec<Target> {
        let has = self
            .list
            .upgrade()
            .is_some_and(|l| l.info(REGISTRY).is_some());
        let registry = if has {
            self.run(caller, REGISTRY, json!({}))
                .await
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        Target::all_from(&registry)
    }

    /// `line` read as quick add, every target's tokens too.
    #[must_use]
    pub fn parse(line: &str, targets: &[Target]) -> Draft {
        parse::parse_with(line, today(), &target::tokens(targets))
    }

    /// Captures `args`: into the inbox, or straight into its target.
    ///
    /// # Errors
    ///
    /// Nothing to capture, an unknown target, or the write failed.
    pub async fn capture(&self, caller: &ActionCaller, args: AddArgs) -> Result<Value, String> {
        let targets = self.targets(caller).await;
        let chips = props_of(args.properties.as_ref());
        let source = args.source.as_deref().unwrap_or("mcp");
        let created = Local::now().to_rfc3339();
        let into = args
            .target
            .as_deref()
            .filter(|t| !t.is_empty() && *t != "inbox");
        let folder = if into == Some(target::NOTE) {
            self.config.notes.clone()
        } else {
            self.config.folder.clone()
        };
        let note = inbox::note_for(&folder, &args.text, &chips, source, &created, |l| {
            Self::parse(l, &targets)
        })
        .ok_or("give `text`: what to capture")?;
        match into {
            None => {
                let path = self
                    .create(caller, &note.path, &note.properties, &note.body)
                    .await?;
                self.changed(&path);
                Ok(json!({
                    "path": path, "title": note.title, "target": "inbox",
                    "properties": note.properties,
                }))
            }
            Some(target::NOTE) => {
                let props = inbox::carried(&note.properties);
                let path = self.create(caller, &note.path, &props, &note.body).await?;
                Ok(
                    json!({ "path": path, "title": note.title, "target": "note", "properties": props }),
                )
            }
            Some(id) => {
                let t = find(&targets, id)?;
                let capture = Capture {
                    title: note.title.clone(),
                    body: note.body.clone(),
                    properties: inbox::carried(&note.properties),
                    source: source.to_owned(),
                    inbox: None,
                };
                self.hand_to(caller, &t, &capture).await
            }
        }
    }

    /// Runs `t`'s action with `capture`: what it made, as `{path, target, …}`.
    async fn hand_to(
        &self,
        caller: &ActionCaller,
        t: &Target,
        capture: &Capture,
    ) -> Result<Value, String> {
        let args = serde_json::to_value(capture).map_err(|e| e.to_string())?;
        let made = self.run(caller, &t.action, args).await?;
        let path = made
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(json!({ "path": path, "title": capture.title, "target": t.id, "made": made }))
    }

    /// A new note at `path`, or ` 2`, ` 3`, … when it is taken: the path
    /// written.
    async fn create(
        &self,
        caller: &ActionCaller,
        path: &str,
        properties: &Map<String, Value>,
        body: &str,
    ) -> Result<String, String> {
        let stem = path.strip_suffix(".md").unwrap_or(path);
        let props = Value::Object(properties.clone()).to_string();
        let mut last = String::new();
        for n in 1..=50_u32 {
            let at = if n == 1 {
                format!("{stem}.md")
            } else {
                format!("{stem} {n}.md")
            };
            match self
                .run(
                    caller,
                    "VAULT_CREATE",
                    json!({ "path": at, "body": body, "properties": props }),
                )
                .await
            {
                Ok(_) => return Ok(at),
                Err(e) if e.contains("exists") || e.contains("taken") => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// The captures in the inbox, oldest first; those put off past today
    /// too when `all`.
    ///
    /// # Errors
    ///
    /// The vault cannot be listed (none is open).
    pub async fn inbox(&self, caller: &ActionCaller, all: bool) -> Result<Vec<Item>, String> {
        let notes = self
            .run(
                caller,
                "VAULT_LIST",
                json!({ "folder": self.config.folder }),
            )
            .await?;
        let today = today();
        let mut items = Vec::new();
        let folder = self.config.folder.trim_matches('/');
        for n in notes.as_array().into_iter().flatten() {
            let Some(path) = n.get("path").and_then(Value::as_str) else {
                continue;
            };
            // Directly in the folder: `Inbox/Processed/…` is not waiting.
            let inside = path
                .strip_prefix(folder)
                .map_or(path, |r| r.trim_start_matches('/'));
            if inside.contains('/') {
                continue;
            }
            let Ok(note) = self
                .run(caller, "VAULT_READ", json!({ "path": path }))
                .await
            else {
                continue;
            };
            if let Some(item) = Item::from_note(&note)
                && (all || item.is_due(today))
            {
                items.push(item);
            }
        }
        items.sort_by(|a, b| a.created.cmp(&b.created).then(a.path.cmp(&b.path)));
        Ok(items)
    }

    /// Capture `path`: a note in the inbox's folder (an `INBOX_*` never
    /// reaches a note outside it).
    async fn item(&self, caller: &ActionCaller, path: &str) -> Result<Item, String> {
        let folder = self.config.folder.trim_matches('/');
        if !folder.is_empty() && !path.starts_with(&format!("{folder}/")) {
            return Err(format!("{path} is not in the inbox ({folder}/)"));
        }
        let note = self
            .run(caller, "VAULT_READ", json!({ "path": path }))
            .await?;
        Item::from_note(&note).ok_or_else(|| format!("{path} is not a capture in the inbox"))
    }

    /// Turns capture `path` into target `id`'s thing; the capture leaves
    /// the inbox.
    ///
    /// # Errors
    ///
    /// No such capture or target, or the target failed (the capture stays).
    pub async fn process(
        &self,
        caller: &ActionCaller,
        path: &str,
        id: &str,
        title: Option<&str>,
        chips: &Map<String, Value>,
    ) -> Result<Value, String> {
        if id == target::NOTE {
            let folder = self.config.notes.clone();
            return self.file(caller, path, &folder, chips).await;
        }
        let targets = self.targets(caller).await;
        let t = find(&targets, id)?;
        let item = self.item(caller, path).await?;
        let capture = Capture {
            title: title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| item.label())
                .to_owned(),
            body: item.body.clone(),
            properties: parse::merge(item.properties.clone(), chips),
            source: item.source.clone(),
            inbox: Some(item.path.clone()),
        };
        let made = self.hand_to(caller, &t, &capture).await?;
        // The capture has become the target's: removing its inbox note is
        // the move's other half, not a deletion the caller asked for (the
        // app's door, so an agent's process is not stopped half done to
        // ask about a note whose words now live in what was made).
        self.run(
            &ActionCaller::app(),
            "VAULT_DELETE",
            json!({ "path": path }),
        )
        .await?;
        self.changed(path);
        Ok(made)
    }

    /// Files capture `path` into `folder` as a plain note: the inbox's
    /// keys taken off, `chips` set.
    ///
    /// # Errors
    ///
    /// No such capture, or the vault refused the move.
    pub async fn file(
        &self,
        caller: &ActionCaller,
        path: &str,
        folder: &str,
        chips: &Map<String, Value>,
    ) -> Result<Value, String> {
        let item = self.item(caller, path).await?;
        for key in inbox::keys::ALL {
            let _ = self
                .run(
                    caller,
                    "VAULT_REMOVE_PROPERTY",
                    json!({ "path": path, "key": key }),
                )
                .await;
        }
        for (key, value) in chips {
            self.set(caller, path, key, value).await?;
        }
        let to = inbox::filed_path(path, folder);
        let at = if to == path {
            to
        } else {
            self.run(caller, "VAULT_RENAME", json!({ "from": path, "to": to }))
                .await?
                .get("to")
                .and_then(Value::as_str)
                .map_or(to, str::to_owned)
        };
        self.changed(path);
        Ok(json!({ "path": at, "title": item.label(), "target": target::NOTE }))
    }

    async fn set(
        &self,
        caller: &ActionCaller,
        path: &str,
        key: &str,
        value: &Value,
    ) -> Result<(), String> {
        if value.is_null() {
            let _ = self
                .run(
                    caller,
                    "VAULT_REMOVE_PROPERTY",
                    json!({ "path": path, "key": key }),
                )
                .await;
            return Ok(());
        }
        self.run(
            caller,
            "VAULT_SET_PROPERTY",
            json!({ "path": path, "key": key, "value": value.to_string() }),
        )
        .await
        .map(|_| ())
    }

    /// Puts capture `path` off until `until` (a date, or a quick-add date
    /// word; tomorrow when `None`).
    ///
    /// # Errors
    ///
    /// No such capture, or a day that is not one.
    pub async fn defer(
        &self,
        caller: &ActionCaller,
        path: &str,
        until: Option<&str>,
    ) -> Result<Value, String> {
        let today = today();
        let day = match until.map(str::trim).filter(|u| !u.is_empty()) {
            None => day_after(1),
            Some(u) => NaiveDate::parse_from_str(u, "%Y-%m-%d")
                .ok()
                .or_else(|| parse::parse(u, today).due)
                .ok_or_else(|| format!("{u:?} is not a day"))?,
        };
        let _ = self.item(caller, path).await?;
        let on = day.format("%Y-%m-%d").to_string();
        self.set(caller, path, inbox::keys::RESURFACE_ON, &json!(on))
            .await?;
        self.changed(path);
        Ok(json!({ "path": path, "resurface_on": on }))
    }

    /// Deletes capture `path`.
    ///
    /// # Errors
    ///
    /// No such capture, or the vault refused.
    pub async fn discard(&self, caller: &ActionCaller, path: &str) -> Result<Value, String> {
        let _ = self.item(caller, path).await?;
        self.run(caller, "VAULT_DELETE", json!({ "path": path }))
            .await?;
        self.changed(path);
        Ok(json!({ "path": path }))
    }
}

fn find(targets: &[Target], id: &str) -> Result<Target, String> {
    targets.iter().find(|t| t.id == id).cloned().ok_or_else(|| {
        let ids: Vec<&str> = targets.iter().map(|t| t.id.as_str()).collect();
        format!("no capture target {id:?} (there are {})", ids.join(", "))
    })
}

/// Registers `CAPTURE_*` and `INBOX_*` on `list`, over its vault and
/// targets; `opener` shows the composer and the deck (none: the two
/// `_OPEN` actions say there is no window). The [`Capturer`] they run.
///
/// # Errors
///
/// An id already on the list.
pub fn register(
    list: &Arc<ActionList>,
    config: Config,
    opener: Option<Opener>,
) -> Result<Capturer, ActionError> {
    let cap = Capturer::new(list, config);
    let o = opener.clone();
    list.register(&OPEN, Effect::Mutating, move |a: OpenArgs| {
        let open = o.as_ref().ok_or("no window to open the composer in")?;
        let text = a.text.unwrap_or_default();
        open(if a.window == Some(true) {
            Open::Window { text }
        } else {
            Open::Composer {
                text,
                target: a.target,
            }
        });
        Ok(())
    })?;
    let c = cap.clone();
    list.register_async_with(
        &ADD,
        Effect::Mutating,
        move |cx: &ActionCaller, a: AddArgs| {
            let (c, cx) = (c.clone(), cx.clone());
            async move { c.capture(&cx, a).await.map(|v| raw(&v)) }
        },
    )?;
    let c = cap.clone();
    list.register_async_with(
        &PARSE,
        Effect::ReadOnly,
        move |cx: &ActionCaller, a: TextArgs| {
            let (c, cx) = (c.clone(), cx.clone());
            async move {
                let targets = c.targets(&cx).await;
                let draft = Capturer::parse(&a.text, &targets);
                let props = draft.properties();
                let suggests =
                    if inbox::suggests_task(&props) && targets.iter().any(|t| t.id == "task") {
                        "task"
                    } else {
                        target::NOTE
                    };
                Ok(raw(
                    &json!({ "title": draft.title, "properties": props, "suggests": suggests }),
                ))
            }
        },
    )?;
    let c = cap.clone();
    list.register_async_with(
        &TARGETS,
        Effect::ReadOnly,
        move |cx: &ActionCaller, (): ()| {
            let (c, cx) = (c.clone(), cx.clone());
            async move {
                let all = c.targets(&cx).await;
                serde_json::to_value(all)
                    .map(|v| raw(&v))
                    .map_err(|e| e.to_string())
            }
        },
    )?;
    let o = opener;
    list.register(&INBOX_OPEN, Effect::Mutating, move |(): ()| {
        let open = o.as_ref().ok_or("no window to open the inbox in")?;
        open(Open::Inbox);
        Ok(())
    })?;
    let c = cap.clone();
    list.register_async_with(
        &INBOX_LIST,
        Effect::ReadOnly,
        move |cx: &ActionCaller, a: ListArgs| {
            let (c, cx) = (c.clone(), cx.clone());
            async move {
                let items = c.inbox(&cx, a.all == Some(true)).await?;
                serde_json::to_value(items)
                    .map(|v| raw(&v))
                    .map_err(|e| e.to_string())
            }
        },
    )?;
    let c = cap.clone();
    list.register_async_with(
        &INBOX_PROCESS,
        Effect::Mutating,
        move |cx: &ActionCaller, a: ProcessArgs| {
            let (c, cx) = (c.clone(), cx.clone());
            async move {
                let chips = props_of(a.properties.as_ref());
                c.process(&cx, &a.path, &a.target, a.title.as_deref(), &chips)
                    .await
                    .map(|v| raw(&v))
            }
        },
    )?;
    let c = cap.clone();
    list.register_async_with(
        &INBOX_FILE,
        Effect::Mutating,
        move |cx: &ActionCaller, a: FileArgs| {
            let (c, cx) = (c.clone(), cx.clone());
            async move {
                let folder = a.folder.unwrap_or_else(|| c.config.notes.clone());
                c.file(&cx, &a.path, &folder, &Map::new())
                    .await
                    .map(|v| raw(&v))
            }
        },
    )?;
    let c = cap.clone();
    list.register_async_with(
        &INBOX_DEFER,
        Effect::Mutating,
        move |cx: &ActionCaller, a: DeferArgs| {
            let (c, cx) = (c.clone(), cx.clone());
            async move {
                c.defer(&cx, &a.path, a.until.as_deref())
                    .await
                    .map(|v| raw(&v))
            }
        },
    )?;
    let c = cap.clone();
    list.register_async_with(
        &INBOX_DISCARD,
        Effect::Destructive,
        move |cx: &ActionCaller, a: PathArgs| {
            let (c, cx) = (c.clone(), cx.clone());
            async move { c.discard(&cx, &a.path).await.map(|v| raw(&v)) }
        },
    )?;
    Ok(cap)
}
