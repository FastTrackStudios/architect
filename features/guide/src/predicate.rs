//! A step's `done`, against the app's Action List (feature `ui`): what
//! the headless runner asserts and what moves a tour on.

use architect::action::{ActionList, Effect, EventKind};

use crate::{Check, Wake, contains, to_json};

/// Why the checks did not pass: not yet (try again), or never.
pub enum Failed {
    /// A check's action gave something else (what it gave, as JSON).
    Not(String, Option<String>),
    /// A check cannot be run: no such action, or one that changes things.
    Hard(String),
}

/// Whether `kind` is one of the events `on` waits for.
pub fn woke(on: &[Wake], kind: &EventKind) -> bool {
    on.iter().any(|w| match (w, kind) {
        (Wake::Ran(want), EventKind::Ran { id, ok }) => *ok && id == want,
        (Wake::Changed(want), EventKind::Changed { topic, .. }) => topic == want,
        _ => false,
    })
}

/// Runs every check on `list`: each a read-only action whose JSON result
/// must contain its `has`.
pub fn checks(list: &ActionList, checks: &[Check]) -> Result<(), Failed> {
    for check in checks {
        let info = list
            .info(&check.id)
            .ok_or_else(|| Failed::Hard(format!("check {}: no such action", check.id)))?;
        if info.effect != Effect::ReadOnly {
            return Err(Failed::Hard(format!(
                "check {}: a check must be a read-only action",
                check.id
            )));
        }
        let args = check
            .args
            .as_ref()
            .map_or_else(|| serde_json::json!({}), to_json);
        let out = list
            .run_json(&check.id, args.to_string().as_bytes())
            .map_err(|e| Failed::Not(format!("check {} failed: {e}", check.id), None))?;
        let actual: serde_json::Value =
            serde_json::from_slice(&out).unwrap_or(serde_json::Value::Null);
        let has = to_json(&check.has);
        if !contains(&actual, &has) {
            return Err(Failed::Not(
                format!("check {} wants {has}", check.id),
                Some(actual.to_string()),
            ));
        }
    }
    Ok(())
}

/// `architect_commands`' action that runs any command by id, a view's own
/// too (`architect_commands::RUN_ACTION`; the runner's test says they are
/// the same).
pub const RUN_COMMAND: &str = "COMMANDS_RUN";

/// What running action `id` with `args` is on `list`: itself, or, for a
/// command the list does not have (a view's own), [`RUN_COMMAND`] with it.
pub fn call(
    list: &ActionList,
    id: &str,
    args: Option<serde_json::Value>,
) -> (String, serde_json::Value) {
    if list.info(id).is_none() && list.info(RUN_COMMAND).is_some() {
        let call = args.map_or_else(
            || serde_json::json!({ "id": id }),
            |a| serde_json::json!({ "id": id, "args": a.to_string() }),
        );
        (RUN_COMMAND.to_owned(), call)
    } else {
        (id.to_owned(), args.unwrap_or_else(|| serde_json::json!({})))
    }
}
