//! A step's `done`, against the app's Action List (feature `ui`): what
//! the headless runner asserts and what moves a tour on.

use architect::action::{ActionList, Effect, EventKind};

use crate::{Check, Wake, contains, to_json};

/// Why the checks did not pass: not yet (try again), or never. A tour
/// only asks whether they passed; the runner (`run`) reads why.
#[cfg_attr(not(feature = "run"), allow(dead_code))]
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
/// must contain its `has`. An async action cannot be one (a tour's checks
/// run on the window's thread): [`checks_waiting`] waits for it.
pub fn checks(list: &ActionList, checks: &[Check]) -> Result<(), Failed> {
    checks_with(list, checks, None)
}

/// [`checks`], an async action (an extension's, its work on a thread of
/// its own) waited for up to `wait`: what the headless runner does.
#[cfg_attr(not(feature = "run"), allow(dead_code))]
pub fn checks_waiting(
    list: &ActionList,
    checks: &[Check],
    wait: std::time::Duration,
) -> Result<(), Failed> {
    checks_with(list, checks, Some(wait))
}

fn checks_with(
    list: &ActionList,
    checks: &[Check],
    wait: Option<std::time::Duration>,
) -> Result<(), Failed> {
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
        let out = run_check(list, &check.id, args.to_string().as_bytes(), wait)
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

/// Runs a check's action: an async one waited for up to `wait`, natively,
/// when there is a wait.
fn run_check(
    list: &ActionList,
    id: &str,
    args: &[u8],
    wait: Option<std::time::Duration>,
) -> Result<Vec<u8>, architect::action::ActionError> {
    #[cfg(not(target_family = "wasm"))]
    if let Some(wait) = wait
        && list.is_async(id)
    {
        return block_on(list.run_json_async(id, args), wait).unwrap_or_else(|| {
            Err(architect::action::ActionError::Failed {
                id: id.to_owned(),
                message: format!("no answer in {wait:?}"),
            })
        });
    }
    #[cfg(target_family = "wasm")]
    let _ = wait;
    list.run_json(id, args)
}

/// Waits for `f` on this thread, up to `wait` (a check's async action does
/// its work on another: an extension's); `None` when it did not finish.
#[cfg(not(target_family = "wasm"))]
fn block_on<F: std::future::Future>(f: F, wait: std::time::Duration) -> Option<F::Output> {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(std::sync::Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    let until = std::time::Instant::now().checked_add(wait)?;
    loop {
        if let std::task::Poll::Ready(out) = f.as_mut().poll(&mut cx) {
            return Some(out);
        }
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        std::thread::park_timeout(left);
    }
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
