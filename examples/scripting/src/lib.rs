//! The scripting example: a small task board whose actions a Luau script
//! lists, queries and runs.
//!
//! Read top to bottom. An app writes three things — its state, the
//! actions over it, who may run which — and then hands its Action List
//! to `architect-scripting`. Everything a script, a saved script, ⌘K or
//! an agent over MCP does goes through that one list. The scripts are in
//! `scripts/`; `main.rs` runs them, and the tests run every one.

use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionError, ActionList, ActionMeta, Effect, Principal};
use architect_mcp::{Server, Tools};
use architect_permissions::{Action, RoleEngine, Rule};
use architect_scripting::actions::Scripting;
use architect_scripting::history::HistoryTransactions;
use architect_scripting::luau::Runtime;
use architect_scripting::{Library, MemoryStore};
use crdt::History;
use crdt::loro::{LoroDoc, LoroMap, LoroValue};
use facet::Facet;

// ── 1. The app's state ────────────────────────────────────────────────
//
// Tasks in a Loro document, with its undo history. Every change is a
// `History::transact`, so each is an undo step of its own — and inside a
// script run, part of the run's one step.

/// One task on the board.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub tag: String,
    pub done: bool,
}

/// The board: a document of tasks and its undo history. Cheap to clone;
/// every clone is the same board.
#[derive(Clone, Debug)]
pub struct Board {
    doc: LoroDoc,
    history: Arc<Mutex<History>>,
}

/// The tasks a fresh board starts with: `(title, tag, done)`.
const SEED: [(&str, &str, bool); 5] = [
    ("Buy milk", "home", false),
    ("Fix the gate", "home", false),
    ("Send the invoice", "work", true),
    ("Review the mix", "studio", false),
    ("Call the plumber", "home", true),
];

impl Board {
    /// A board with five tasks (`t1`…`t5`), two of them done, and no undo
    /// steps.
    ///
    /// # Errors
    ///
    /// When the document refuses a change (it does not).
    pub fn new() -> Result<Self, String> {
        let doc = LoroDoc::new();
        for (title, tag, done) in SEED {
            let id = insert(&doc, title, tag)?;
            if done {
                task_map(&doc, &id)?
                    .insert("done", true)
                    .map_err(|e| e.to_string())?;
            }
        }
        doc.commit();
        // Made after the seed, so the seed is no step to undo.
        let history = Arc::new(Mutex::new(History::new(&doc, 100)));
        Ok(Self { doc, history })
    }

    /// Every task, in the order they were added.
    #[must_use]
    pub fn tasks(&self) -> Vec<Task> {
        let LoroValue::Map(tasks) = self.doc.get_map("tasks").get_deep_value() else {
            return Vec::new();
        };
        let mut found: Vec<(i64, Task)> = tasks
            .iter()
            .filter_map(|(id, value)| {
                let LoroValue::Map(fields) = value else {
                    return None;
                };
                let text = |key: &str| match fields.get(key) {
                    Some(LoroValue::String(s)) => s.to_string(),
                    _ => String::new(),
                };
                let n = match fields.get("n") {
                    Some(LoroValue::I64(n)) => *n,
                    _ => 0,
                };
                let task = Task {
                    id: id.clone(),
                    title: text("title"),
                    tag: text("tag"),
                    done: matches!(fields.get("done"), Some(LoroValue::Bool(true))),
                };
                Some((n, task))
            })
            .collect();
        found.sort_by_key(|(n, _)| *n);
        found.into_iter().map(|(_, task)| task).collect()
    }

    /// Adds a task, as an undo step named "Add task": its id.
    ///
    /// # Errors
    ///
    /// When the document refuses the change.
    pub fn add(&self, title: &str, tag: &str) -> Result<String, String> {
        self.change("Add task", |doc| insert(doc, title, tag))
    }

    /// Marks task `id` done, as an undo step named "Complete task".
    ///
    /// # Errors
    ///
    /// When there is no such task.
    pub fn complete(&self, id: &str) -> Result<(), String> {
        let task = task_map(&self.doc, id)?;
        self.change("Complete task", |_| {
            task.insert("done", true).map_err(|e| e.to_string())
        })
    }

    /// Takes task `id` off the board, as an undo step named "Remove task".
    ///
    /// # Errors
    ///
    /// When there is no such task.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        task_map(&self.doc, id)?;
        self.change("Remove task", |doc| {
            doc.get_map("tasks").delete(id).map_err(|e| e.to_string())
        })
    }

    /// Undoes the last step: its name.
    #[must_use]
    pub fn undo(&self) -> Option<String> {
        self.history().undo()
    }

    /// Every step Undo can take back, oldest first.
    #[must_use]
    pub fn steps(&self) -> Vec<String> {
        self.history().steps().to_vec()
    }

    /// The board's undo as a script run sees it: each run one step of this
    /// history, taken back when it fails.
    #[must_use]
    pub fn transactions(&self) -> HistoryTransactions {
        HistoryTransactions::new(self.doc.clone(), Arc::clone(&self.history))
    }

    /// `change` as one step named `name`. The history is locked only for
    /// the change itself, never across a call back into the Action List.
    fn change<R>(&self, name: &str, change: impl FnOnce(&LoroDoc) -> R) -> R {
        self.history().transact(&self.doc, name, change)
    }

    fn history(&self) -> std::sync::MutexGuard<'_, History> {
        self.history.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Puts a task in `doc`, not done, under the next id (`t<n>`; an id is
/// never given twice, even after its task is removed): its id.
fn insert(doc: &LoroDoc, title: &str, tag: &str) -> Result<String, String> {
    let board = doc.get_map("board");
    let n = match board.get("next").and_then(|v| v.into_value().ok()) {
        Some(LoroValue::I64(n)) => n,
        _ => 1,
    };
    board
        .insert("next", n.saturating_add(1))
        .map_err(|e| e.to_string())?;
    let id = format!("t{n}");
    let task = doc
        .get_map("tasks")
        .insert_container(&id, LoroMap::new())
        .map_err(|e| e.to_string())?;
    for (key, value) in [
        ("title", LoroValue::from(title)),
        ("tag", LoroValue::from(tag)),
        ("done", LoroValue::from(false)),
        ("n", LoroValue::from(n)),
    ] {
        task.insert(key, value).map_err(|e| e.to_string())?;
    }
    Ok(id)
}

fn task_map(doc: &LoroDoc, id: &str) -> Result<LoroMap, String> {
    doc.get_map("tasks")
        .get(id)
        .and_then(|v| v.into_container().ok())
        .and_then(|c| c.into_map().ok())
        .ok_or_else(|| format!("there is no task {id}"))
}

// ── 2. The actions ────────────────────────────────────────────────────
//
// Four actions on the Action List: one read-only (a script reads it with
// `state.query`), three that change the board (`actions.run`). What a key,
// a menu, ⌘K, a script and a model run are these same four.

const fn meta(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Tasks",
        method_name: id,
        display_name,
        description,
        category: "Tasks",
        group: "Tasks",
        shortcut: "",
        toggleable: false,
        undo: true,
    }
}

pub static LIST: ActionMeta = meta(
    "TASKS_LIST",
    "List tasks",
    "The tasks on the board, in the order they were added, filtered by `tag` and `open`.",
);
pub static ADD: ActionMeta = meta(
    "TASKS_ADD",
    "Add a task",
    "Adds a task titled `title`, tagged `tag` (\"inbox\" without one): its id.",
);
pub static COMPLETE: ActionMeta = meta("TASKS_COMPLETE", "Complete a task", "Marks a task done.");
pub static REMOVE: ActionMeta = meta(
    "TASKS_REMOVE",
    "Remove a task",
    "Takes a task off the board.",
);

/// Which tasks `TASKS_LIST` gives.
#[derive(Debug, Default, Facet)]
pub struct Filter {
    /// Only the tasks with this tag.
    pub tag: Option<String>,
    /// Only the open tasks (true) or only the done ones (false).
    pub open: Option<bool>,
}

/// `TASKS_ADD`'s arguments.
#[derive(Debug, Facet)]
pub struct NewTask {
    /// What the task says.
    pub title: String,
    /// Its tag; "inbox" when it has none.
    pub tag: Option<String>,
}

/// What `TASKS_ADD` answers.
#[derive(Debug, Facet)]
pub struct Added {
    /// The new task's id.
    pub id: String,
}

/// One task, by id: `TASKS_COMPLETE`'s and `TASKS_REMOVE`'s arguments.
#[derive(Debug, Facet)]
pub struct TaskId {
    /// The task's id (`t1`).
    pub id: String,
}

/// Puts the board's four actions on `list`.
///
/// # Errors
///
/// When an id is already on it.
pub fn register(board: &Board, list: &ActionList) -> Result<(), ActionError> {
    let b = board.clone();
    list.register(&LIST, Effect::ReadOnly, move |f: Filter| {
        Ok(b.tasks()
            .into_iter()
            .filter(|t| f.tag.as_deref().is_none_or(|tag| t.tag == tag))
            .filter(|t| f.open.is_none_or(|open| t.done != open))
            .collect::<Vec<Task>>())
    })?;
    let b = board.clone();
    list.register(&ADD, Effect::Mutating, move |a: NewTask| {
        let tag = a.tag.as_deref().unwrap_or("inbox");
        b.add(&a.title, tag).map(|id| Added { id })
    })?;
    let b = board.clone();
    list.register(&COMPLETE, Effect::Mutating, move |a: TaskId| {
        b.complete(&a.id)
    })?;
    let b = board.clone();
    list.register(&REMOVE, Effect::Destructive, move |a: TaskId| {
        b.remove(&a.id)
    })?;
    Ok(())
}

// ── 3. Who may run what ───────────────────────────────────────────────
//
// The list's authorizer is asked before every run by anyone but the app
// itself — and a script runs each action as whoever ran the script. An
// editor may run everything; a viewer may read the board and run
// scripts, so a viewer's script that tries to change the board is
// refused at that line. Services (the agent) are trusted.

/// Ana, who may run every action.
#[must_use]
pub fn editor() -> Principal {
    Principal::User {
        user_id: "ana".into(),
    }
}

/// Vic, who may read the board and run scripts, and change nothing.
#[must_use]
pub fn viewer() -> Principal {
    Principal::User {
        user_id: "vic".into(),
    }
}

/// The agent on the other end of MCP.
#[must_use]
pub fn agent() -> Principal {
    Principal::Service {
        name: "agent".into(),
    }
}

/// The board's roles: `editor` runs every action, `viewer` only the
/// read-only ones and `SCRIPT_RUN`.
#[must_use]
pub fn authorizer() -> RoleEngine {
    let run = &[Action::RUN];
    let mut engine = RoleEngine::new();
    engine.set_role("editor", vec![Rule::new("action/**", run)]);
    engine.set_role(
        "viewer",
        ["TASKS_LIST", "SCRIPT_RUN", "SCRIPT_DESCRIBE", "SCRIPT_LIST"]
            .iter()
            .map(|id| Rule::new(format!("action/{id}"), run))
            .collect(),
    );
    engine.set_member("ana", "editor");
    engine.set_member("vic", "viewer");
    engine
}

// ── 4. Scripting ──────────────────────────────────────────────────────
//
// `Scripting` puts the SCRIPT_* actions on the list (run, save, describe,
// list, delete) and every saved script as an action of its own. With
// the board's history as its transactions, a run is one undo step.

/// The whole app: the board, its Action List, and scripting over it.
#[derive(Clone, Debug)]
pub struct App {
    pub board: Board,
    pub list: ActionList,
    pub scripting: Scripting,
}

impl App {
    /// A fresh board, its actions and authorizer, and scripting installed.
    /// Saved scripts are kept in memory, the person's and the
    /// organization's; an app on a desk keeps them in a settings file
    /// (`architect_scripting::settings::SettingsStore`).
    ///
    /// # Errors
    ///
    /// When an action could not be registered.
    pub fn new() -> Result<Self, String> {
        let board = Board::new()?;
        let list = ActionList::new();
        register(&board, &list).map_err(|e| e.to_string())?;
        list.set_authorizer(authorizer());
        let library = Library::new(MemoryStore::new()).with_org(MemoryStore::new());
        let scripting =
            Scripting::new(list.clone(), library).with_transactions(board.transactions());
        scripting.install().map_err(|e| e.to_string())?;
        Ok(Self {
            board,
            list,
            scripting,
        })
    }

    /// A runtime for live scripts (`Runtime::start`): each frame one step
    /// of the board's history.
    #[must_use]
    pub fn runtime(&self) -> Runtime {
        Runtime::new(self.list.clone()).with_transactions(Arc::new(self.board.transactions()))
    }

    /// The MCP server a model drives the board through, as `who`: every
    /// action a tool of its own (`tasks_list`, `script_run`,
    /// `script_save`, and each saved script as it is saved).
    #[must_use]
    pub fn mcp(&self, who: Principal) -> Server {
        Server::new("scripting-example", "0", self.list.clone())
            .with_tools(Tools::Named)
            .with_principal(who)
            .with_instructions(
                "A task board. Call script_describe to see what a Luau script can call, \
                 script_run to run one as a single undo step, script_save to keep it as an \
                 action of its own.",
            )
    }
}
