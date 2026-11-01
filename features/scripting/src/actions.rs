//! Scripts on the Action List: the `SCRIPT_*` actions, and every saved
//! script an action of its own.
//!
//! | action | what it does |
//! |---|---|
//! | `SCRIPT_RUN` | runs a script once: `{source, args?, name?}` → `{result, log, ran, undo}` |
//! | `SCRIPT_DESCRIBE` | the API a script has, and every action it may run with its schemas |
//! | `SCRIPT_LIST` | the saved scripts, with their action ids |
//! | `SCRIPT_SAVE` / `SCRIPT_SAVE_ORG` | saves one, the person's / the organization's: checked, then an action |
//! | `SCRIPT_DELETE` / `SCRIPT_DELETE_ORG` | forgets one, and takes its action off the list |
//!
//! A model over MCP (`architect-mcp`) gets these as it gets every action
//! — `run` with `SCRIPT_RUN`, or the tools `script_run`, `script_save`,
//! `script_describe` under `Tools::Named` — and its scripts run as it:
//! every action one runs is authorized for the model on its own. The
//! organization's are their own actions so a permission engine can let a
//! person save their own scripts but not the organization's.
//!
//! A saved script's action (`SCRIPT_MY_…`, `SCRIPT_ORG_…`) takes nothing,
//! so a key, ⌘K and the ribbon run it; it reads the script when it runs,
//! so what was saved last is what runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use facet::Facet;
use serde_json::{Value, json};

use crate::luau::{Limits, Report, Runtime, detail};
use crate::{Library, LibraryError, Scope, Script, Transactions};

/// How many runs may be under way at once, scripts run by scripts
/// included: what stops a saved script that runs itself.
pub const MAX_RUNNING: usize = 16;

const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Scripting",
        method_name: id,
        display_name: name,
        description,
        category: "Scripts",
        group: "Scripts",
        shortcut: "",
        toggleable: false,
        undo: true,
    }
}

static RUN: ActionMeta = meta(
    "SCRIPT_RUN",
    "Run a script",
    "Runs a Luau script once, as one undo step: `source` is the script, `args` JSON it reads as \
     `args`, `name` names the undo step. Answers with what it returned, what it printed and the \
     actions it ran. SCRIPT_DESCRIBE says what a script can call.",
);
static DESCRIBE: ActionMeta = meta(
    "SCRIPT_DESCRIBE",
    "Describe the script API",
    "What a Luau script can call (actions.list, actions.describe, actions.run, actions.enabled, \
     state.query, print, args) and every action it may run, with what each takes and gives.",
);
static LIST: ActionMeta = meta(
    "SCRIPT_LIST",
    "List saved scripts",
    "Every saved script — yours and your organization's — with its action id.",
);
static SAVE: ActionMeta = meta(
    "SCRIPT_SAVE",
    "Save a script",
    "Saves a Luau script as one of yours under `name`: checked first, then an action of its own \
     (SCRIPT_MY_<NAME>), in the action list and ⌘K, bindable. Saving a name again replaces it.",
);
static SAVE_ORG: ActionMeta = meta(
    "SCRIPT_SAVE_ORG",
    "Save a script for the organization",
    "As SCRIPT_SAVE, for everyone in the organization (SCRIPT_ORG_<NAME>).",
);
static DELETE: ActionMeta = meta(
    "SCRIPT_DELETE",
    "Delete a script",
    "Forgets one of your saved scripts and takes its action off the list.",
);
static DELETE_ORG: ActionMeta = meta(
    "SCRIPT_DELETE_ORG",
    "Delete an organization script",
    "Forgets one of the organization's saved scripts and takes its action off the list.",
);

/// `SCRIPT_RUN`'s arguments.
#[derive(Debug, Facet)]
pub struct RunArgs {
    /// The Luau source.
    pub source: String,
    /// What the script reads as `args`, as JSON text.
    pub args: Option<String>,
    /// The undo step's name ("Script: <name>"; "Script" without one).
    pub name: Option<String>,
}

/// `SCRIPT_SAVE`'s and `SCRIPT_SAVE_ORG`'s arguments.
#[derive(Debug, Facet)]
pub struct SaveArgs {
    pub name: String,
    /// The Luau source.
    pub source: String,
    /// What it does, for the action list and a model.
    pub description: Option<String>,
}

/// What `SCRIPT_SAVE` answers.
#[derive(Debug, Facet)]
pub struct Saved {
    /// The action it runs as.
    pub id: String,
}

/// `SCRIPT_DELETE`'s and `SCRIPT_DELETE_ORG`'s arguments.
#[derive(Debug, Facet)]
pub struct DeleteArgs {
    pub name: String,
}

/// What `SCRIPT_DELETE` answers.
#[derive(Debug, Facet)]
pub struct Deleted {
    /// The action taken off the list; `None` when there was no such script.
    pub id: Option<String>,
}

/// One saved script, as `SCRIPT_LIST` gives it.
#[derive(Debug, Facet)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub description: String,
    pub scope: Scope,
    pub source: String,
}

/// What `SCRIPT_LIST` answers.
#[derive(Debug, Facet)]
pub struct Listing {
    pub scripts: Vec<Listed>,
}

/// Why scripting could not be put on a list.
#[derive(Debug)]
pub enum InstallError {
    /// An action could not be registered (its id is taken).
    Action(ActionError),
    /// The saved scripts could not be read.
    Library(LibraryError),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Action(e) => e.fmt(f),
            Self::Library(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<ActionError> for InstallError {
    fn from(e: ActionError) -> Self {
        Self::Action(e)
    }
}

impl From<LibraryError> for InstallError {
    fn from(e: LibraryError) -> Self {
        Self::Library(e)
    }
}

/// Scripting for one Action List: its runtime, its saved scripts, and the
/// actions that reach them. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Scripting {
    runtime: Runtime,
    library: Arc<Library>,
    /// How many runs are under way now, nested ones included.
    running: Arc<AtomicUsize>,
}

impl Scripting {
    /// Scripting over `list`, keeping saved scripts in `library`. Nothing
    /// is on the list until [`Self::install`].
    #[must_use]
    pub fn new(list: ActionList, library: Library) -> Self {
        Self {
            runtime: Runtime::new(list),
            library: Arc::new(library),
            running: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Every run one step of `transactions` (the document's undo), taken
    /// back when it fails.
    #[must_use]
    pub fn with_transactions(mut self, transactions: impl Transactions + 'static) -> Self {
        self.runtime = self.runtime.with_transactions(Arc::new(transactions));
        self
    }

    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.runtime = self.runtime.with_limits(limits);
        self
    }

    /// The saved scripts.
    #[must_use]
    pub fn library(&self) -> &Library {
        &self.library
    }

    const fn list(&self) -> &ActionList {
        self.runtime.list()
    }

    /// Puts the `SCRIPT_*` actions on the list, and every saved script
    /// (the organization's ones only when the library keeps them).
    ///
    /// # Errors
    ///
    /// An id already taken, or saved scripts that cannot be read.
    pub fn install(&self) -> Result<(), InstallError> {
        let list = self.list();
        let me = self.clone();
        list.register_async_with(
            &RUN,
            Effect::Mutating,
            move |caller: &ActionCaller, a: RunArgs| {
                let (me, caller) = (me.clone(), caller.clone());
                async move { me.run_once(&caller, a).await }
            },
        )?;
        let me = self.clone();
        list.register_with(
            &DESCRIBE,
            Effect::ReadOnly,
            move |caller: &ActionCaller, (): ()| Ok(RawJson::from(me.describe(caller).to_string())),
        )?;
        let me = self.clone();
        list.register(&LIST, Effect::ReadOnly, move |(): ()| me.listing())?;
        for (meta, scope) in [(&SAVE, Scope::Person), (&SAVE_ORG, Scope::Org)] {
            if !self.library.keeps(scope) {
                continue;
            }
            let me = self.clone();
            list.register(meta, Effect::Mutating, move |a: SaveArgs| {
                let script = Script {
                    name: a.name,
                    description: a.description.unwrap_or_default(),
                    source: a.source,
                    scope,
                };
                me.save(script).map(|id| Saved { id })
            })?;
        }
        for (meta, scope) in [(&DELETE, Scope::Person), (&DELETE_ORG, Scope::Org)] {
            if !self.library.keeps(scope) {
                continue;
            }
            let me = self.clone();
            list.register(meta, Effect::Destructive, move |a: DeleteArgs| {
                me.delete(scope, &a.name).map(|id| Deleted { id })
            })?;
        }
        for script in self.library.scripts()? {
            if let Some(id) = script.id() {
                self.register_script(&script, &id)?;
            }
        }
        Ok(())
    }

    /// Runs `source` once as `caller`, as one undo step named `step`.
    pub async fn run(
        &self,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
    ) -> Report {
        let Some(_running) = Running::enter(&self.running) else {
            return Report {
                result: Err(crate::ScriptError::new(
                    crate::ErrorKind::Limit,
                    None,
                    format!("{MAX_RUNNING} scripts are running already (one that runs itself?)"),
                )),
                log: Vec::new(),
                ran: Vec::new(),
                undo: None,
            };
        };
        self.runtime.run(caller, source, args, step).await
    }

    /// Checks `script`, keeps it, and puts it on the list as its own
    /// action (in place of the one it replaces): its action id.
    ///
    /// # Errors
    ///
    /// It is not Luau (the line says where), its name makes no id, or it
    /// cannot be kept.
    pub fn save(&self, script: Script) -> Result<String, String> {
        Runtime::check(&script.source).map_err(|e| e.to_string())?;
        let saved = script.clone();
        let id = self.library.save(script).map_err(|e| e.to_string())?;
        self.list().unregister(&id);
        self.register_script(&saved, &id)
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Forgets `scope`'s script `name` and takes its action off the list:
    /// the action's id, when there was one.
    ///
    /// # Errors
    ///
    /// When it cannot be forgotten.
    pub fn delete(&self, scope: Scope, name: &str) -> Result<Option<String>, String> {
        let id = self
            .library
            .delete(scope, name)
            .map_err(|e| e.to_string())?;
        if let Some(id) = &id {
            self.list().unregister(id);
        }
        Ok(id)
    }

    /// The API a script has, and the actions `caller` may run from one.
    #[must_use]
    pub fn describe(&self, caller: &ActionCaller) -> Value {
        let infos = caller
            .principal()
            .map_or_else(|| self.list().infos(), |p| self.list().infos_for(p));
        json!({
            "language": "Luau (https://luau.org): Lua 5.1 with types, sandboxed",
            "globals": {
                "actions.list(filter?)": "the actions you may run, filtered by text: {id, name, description, category, group, effect, enabled, disabled_reason}",
                "actions.describe(id)": "one action with `params` and `result` (JSON Schema as tables), or nil",
                "actions.run(id, args?)": "runs an action with `args` (a table) and answers with its result; raises an error when it fails (catch it with pcall)",
                "actions.enabled(id)": "whether it can run now, and why not",
                "state.query(id, args?)": "runs a read-only action; refuses one that changes the app",
                "print(...) / log(...)": "a line in the run's log",
                "args": "what the run was given, or nil",
            },
            "returns": "what the script returns is the run's result (JSON)",
            "undo": "a run is one undo step; a run that fails is taken back",
            "errors": "a failure says its line: {kind: syntax|runtime|action|limit, line, message}",
            "example": "local selected = state.query(\"TRACKS_SELECTED\")\nfor _, id in selected do\n  actions.run(\"TRACKS_ARM\", { id = id, on = true })\nend\nreturn #selected",
            "actions": infos.iter().map(detail).collect::<Vec<_>>(),
        })
    }

    fn listing(&self) -> Result<Listing, String> {
        let scripts = self.library.scripts().map_err(|e| e.to_string())?;
        Ok(Listing {
            scripts: scripts
                .into_iter()
                .filter_map(|s| {
                    Some(Listed {
                        id: s.id()?,
                        name: s.name,
                        description: s.description,
                        scope: s.scope,
                        source: s.source,
                    })
                })
                .collect(),
        })
    }

    async fn run_once(&self, caller: &ActionCaller, a: RunArgs) -> Result<RawJson, String> {
        let args = match a.args.as_deref().map(str::trim) {
            None | Some("") => Value::Null,
            Some(text) => {
                serde_json::from_str(text).map_err(|e| format!("`args` is not JSON: {e}"))?
            }
        };
        let step = a
            .name
            .as_deref()
            .map_or_else(|| "Script".to_owned(), |name| format!("Script: {name}"));
        self.answer(&self.run(caller, &a.source, args, &step).await)
    }

    async fn run_saved(&self, caller: &ActionCaller, id: &str) -> Result<RawJson, String> {
        let script = self
            .library
            .find(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{id} is no longer saved"))?;
        let step = format!("Script: {}", script.name);
        self.answer(&self.run(caller, &script.source, Value::Null, &step).await)
    }

    /// A run as an action's answer: the report, or — when it failed —
    /// what went wrong, where, what it ran and printed.
    fn answer(&self, report: &Report) -> Result<RawJson, String> {
        let Err(error) = &report.result else {
            return Ok(RawJson::from(report.to_json().to_string()));
        };
        let mut message = error.to_string();
        if !report.ran.is_empty() {
            message.push_str("\nran: ");
            message.push_str(&report.ran.join(", "));
            if self.runtime.has_transactions() {
                message.push_str(" (what they changed was taken back)");
            }
        }
        if !report.log.is_empty() {
            message.push_str("\nlog:\n");
            message.push_str(&report.log.join("\n"));
        }
        Err(message)
    }

    /// Puts saved `script` on the list as action `id`.
    fn register_script(&self, script: &Script, id: &str) -> Result<(), ActionError> {
        let kept = |text: &str| -> &'static str { Box::leak(text.to_owned().into_boxed_str()) };
        let description = if script.description.is_empty() {
            kept(&format!("Runs the script “{}”.", script.name))
        } else {
            kept(&script.description)
        };
        // An action's description lives as long as the list does: a few
        // bytes a save, as `ActionList::register_forwarded` keeps them.
        let meta: &'static ActionMeta = Box::leak(Box::new(ActionMeta {
            id: kept(id),
            trait_name: "",
            method_name: kept(id),
            display_name: kept(&script.name),
            description,
            category: "Scripts",
            group: script.scope.group(),
            shortcut: "",
            toggleable: false,
            undo: true,
        }));
        let me = self.clone();
        self.list().register_async_with(
            meta,
            Effect::Mutating,
            move |caller: &ActionCaller, (): ()| {
                let (me, caller) = (me.clone(), caller.clone());
                async move { me.run_saved(&caller, meta.id).await }
            },
        )
    }
}

/// One run under way; the count falls when it ends.
struct Running<'a>(&'a AtomicUsize);

impl<'a> Running<'a> {
    fn enter(running: &'a AtomicUsize) -> Option<Self> {
        let was = running.fetch_add(1, Ordering::SeqCst);
        if was >= MAX_RUNNING {
            running.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self(running))
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
