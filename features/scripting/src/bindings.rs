//! What a script's `actions` and `state` globals do, apart from Luau
//! (feature `bindings`, wasm-clean): which actions a run sees, what
//! `actions.list` and `actions.describe` give, why one is refused, and a
//! run's error as a [`ScriptError`].
//!
//! The one implementation of these globals. The native runtime
//! ([`crate::luau`]) calls it from its mlua functions; a browser's Luau
//! (`architect_pages::web::luau`, where the VM is in a Web Worker and
//! each global is a message) calls it on the page for each message.

use architect::action::{ActionCaller, ActionInfo, ActionList, Effect};
use serde_json::{Value, json};

use crate::{ErrorKind, ScriptError};

/// The chunk name a script's errors are reported under (`script:3: …`).
pub const CHUNK: &str = "script";

/// What a run may reach, past what its caller may: `Err(why)` for an
/// action it may not see or run. An extension's capabilities (the action
/// namespaces it named, the most it may change) are one.
pub type Gate = std::sync::Arc<dyn Fn(&ActionInfo) -> Result<(), String> + Send + Sync>;

/// `kind`'s name in a report's JSON (`{error: {kind}}`).
#[must_use]
pub const fn kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Syntax => "syntax",
        ErrorKind::Runtime => "runtime",
        ErrorKind::Action => "action",
        ErrorKind::Limit => "limit",
    }
}

/// `effect`'s name, as `actions.list` gives it.
#[must_use]
pub const fn effect_name(effect: Effect) -> &'static str {
    match effect {
        Effect::ReadOnly => "read_only",
        Effect::Mutating => "mutating",
        Effect::Destructive => "destructive",
    }
}

/// Why a run will not run `id`: its gate keeps it out, or — for
/// `state.query` — it changes the app.
#[must_use]
pub fn refusal(
    list: &ActionList,
    id: &str,
    read_only: bool,
    gate: Option<&Gate>,
) -> Option<String> {
    let info = list.info(id)?;
    if let Some(Err(why)) = gate.map(|g| g(&info)) {
        return Some(format!("{id}: {why}"));
    }
    if !read_only {
        return None;
    }
    (info.effect != Effect::ReadOnly).then(|| {
        format!(
            "{id} changes the app ({}): run it with actions.run",
            effect_name(info.effect)
        )
    })
}

/// The actions `who` may run, and the gate lets through.
#[must_use]
pub fn visible(list: &ActionList, who: &ActionCaller, gate: Option<&Gate>) -> Vec<ActionInfo> {
    let mut infos = who
        .principal()
        .map_or_else(|| list.infos(), |p| list.infos_for(p));
    if let Some(gate) = gate {
        infos.retain(|i| gate(i).is_ok());
    }
    infos
}

fn matches(info: &ActionInfo, filter: &str) -> bool {
    [
        &info.id,
        &info.display_name,
        &info.description,
        &info.category,
        &info.group,
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(filter))
}

/// One action, as `actions.list` gives it.
#[must_use]
pub fn summary(info: &ActionInfo) -> Value {
    json!({
        "id": info.id,
        "name": info.display_name,
        "description": info.description,
        "category": info.category,
        "group": info.group,
        "effect": effect_name(info.effect),
        "enabled": info.enabled,
        "disabled_reason": info.disabled_reason,
    })
}

/// One action with what it takes and gives, as `actions.describe` gives it.
#[must_use]
pub fn detail(info: &ActionInfo) -> Value {
    let schema = |s: &Option<String>| {
        s.as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
    };
    let mut out = summary(info);
    if let Value::Object(map) = &mut out {
        map.insert("params".into(), schema(&info.params_schema).into());
        map.insert("result".into(), schema(&info.result_schema).into());
    }
    out
}

/// `actions.list(filter?)`: the actions the caller sees, filtered by text.
#[must_use]
pub fn list(
    list: &ActionList,
    who: &ActionCaller,
    gate: Option<&Gate>,
    filter: Option<&str>,
) -> Value {
    let filter = filter.map(str::to_lowercase);
    Value::Array(
        visible(list, who, gate)
            .iter()
            .filter(|i| filter.as_deref().is_none_or(|f| matches(i, f)))
            .map(summary)
            .collect(),
    )
}

/// `actions.describe(id)`: one, with its params and result; `null` for
/// none it sees.
#[must_use]
pub fn describe(list: &ActionList, who: &ActionCaller, gate: Option<&Gate>, id: &str) -> Value {
    visible(list, who, gate)
        .into_iter()
        .find(|i| i.id == id)
        .map_or(Value::Null, |info| detail(&info))
}

/// `actions.enabled(id)`: whether it can run now, and why not.
#[must_use]
pub fn enabled(
    list: &ActionList,
    who: &ActionCaller,
    gate: Option<&Gate>,
    id: &str,
) -> (bool, Option<String>) {
    visible(list, who, gate)
        .into_iter()
        .find(|i| i.id == id)
        .map_or_else(
            || (false, Some(format!("no action {id}"))),
            |i| (i.enabled, i.disabled_reason),
        )
}

/// `actions.run(id, args?)` — or, `read_only`, `state.query` — on this
/// thread's runtime: its answer as JSON, or why it was refused or failed
/// (an [`ErrorKind::Action`] at the script's line).
///
/// # Errors
///
/// The gate refuses it, `state.query` was asked to change the app, or
/// the action failed.
pub async fn run(
    list: &ActionList,
    who: &ActionCaller,
    gate: Option<&Gate>,
    id: &str,
    args: Option<&Value>,
    read_only: bool,
) -> Result<Value, String> {
    if let Some(message) = refusal(list, id, read_only, gate) {
        return Err(message);
    }
    let bytes = args.map(|a| a.to_string().into_bytes()).unwrap_or_default();
    let out = list
        .run_json_async_with(who, id, &bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::from_slice(&out).unwrap_or(Value::Null))
}

/// `caller` as a script's: its invocation's source is `Script`, the rest
/// of it (and the principal) as it was.
#[must_use]
pub fn as_script(caller: &ActionCaller) -> ActionCaller {
    let mut invocation = caller.invocation().cloned().unwrap_or_default();
    invocation.source = architect::action::InvocationSource::Script;
    caller.clone().with_invocation(invocation)
}

/// `script:3: attempt to …` as line 3 and its message; the traceback
/// after the first line is left out.
#[must_use]
pub fn located(kind: ErrorKind, message: &str) -> ScriptError {
    let first = message.lines().next().unwrap_or_default();
    let mut rest = first;
    // A nested error repeats the prefix (`script:3: script:5: …`): the
    // innermost line is where it happened.
    let mut line = None;
    while let Some((at, tail)) = rest
        .strip_prefix(CHUNK)
        .and_then(|r| r.strip_prefix(':'))
        .and_then(|r| r.split_once(':'))
        .and_then(|(n, tail)| n.parse::<u32>().ok().map(|n| (n, tail)))
    {
        line = Some(at);
        rest = tail.trim_start();
    }
    ScriptError::new(kind, line, rest)
}

#[cfg(test)]
mod tests {
    use super::located;
    use crate::ErrorKind;

    /// A script started from ⌘K runs its inner calls as a script: the
    /// palette's press is not handed on, the principal and the rest stay.
    #[test]
    fn a_scripts_inner_calls_are_the_scripts_not_the_palettes() {
        use architect::action::{ActionCaller, Invocation, InvocationSource, Principal};
        let mut pressed = Invocation::new(InvocationSource::Palette);
        pressed.pane = Some("p1".to_owned());
        let person = ActionCaller::app().with_invocation(pressed);
        let inner = super::as_script(&person);
        let got = inner.invocation().expect("an invocation");
        assert_eq!(got.source, InvocationSource::Script);
        assert_eq!(got.pane.as_deref(), Some("p1"));
        assert!(inner.principal().is_none());
        // An untagged caller gets one; a principal stays.
        assert_eq!(
            super::as_script(&ActionCaller::app())
                .invocation()
                .map(|i| i.source),
            Some(InvocationSource::Script)
        );
        let agent = ActionCaller::new(Principal::extension("acme".to_owned(), None));
        assert!(super::as_script(&agent).principal().is_some());
    }

    #[test]
    fn a_message_gives_up_its_line() {
        let e = located(
            ErrorKind::Runtime,
            "script:3: attempt to add\nstack traceback:\n…",
        );
        assert_eq!(e.line, Some(3));
        assert_eq!(e.message, "attempt to add");
        let e = located(ErrorKind::Runtime, "boom");
        assert_eq!((e.line, e.message.as_str()), (None, "boom"));
    }
}
