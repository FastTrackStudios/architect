//! The Board's actions its page runs, in a browser: `LAB_BOARD_STATE`,
//! `LAB_BOARD_PIN` and `LAB_BOARD_CLEAR`, as `apps/sandbox/board/src/lib.rs`
//! `Board` answers them (its Rust extension, a linked extension and a
//! process, is not built for a browser). Its manifest says what they are.

use std::cell::RefCell;
use std::sync::Arc;

use architect::action::{ActionError, ActionFuture, ActionInfo, ActionList, Effect};
use architect_extensions::Manifest;
use architect_extensions::manifest::Reach;
use serde_json::{Value, json};

thread_local! {
    static PINS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The board, as `LAB_BOARD_STATE` says it.
fn snapshot() -> Value {
    PINS.with(|p| {
        json!({ "pins": *p.borrow(), "chart": null, "issues": [], "tempo": 120, "last_invoked": null, "process": 0 })
    })
}

/// As its manifest declares it: its name, its effect, its parameters.
fn info(manifest: &Manifest, id: &str) -> ActionInfo {
    let decl = manifest.actions.iter().find(|d| d.id == id);
    let effect = match decl.map(|d| d.effect) {
        Some(Reach::Mutating) => Effect::Mutating,
        Some(Reach::Destructive) => Effect::Destructive,
        _ => Effect::ReadOnly,
    };
    ActionInfo {
        id: id.to_owned(),
        display_name: decl.map_or_else(|| id.to_owned(), |d| d.name.clone()),
        description: String::new(),
        category: "Extensions".to_owned(),
        group: "Board".to_owned(),
        shortcut: String::new(),
        effect,
        params_schema: decl.and_then(|d| d.params.clone()),
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    }
}

fn answer(f: fn(&Value) -> Value) -> architect::action::AsyncHandler {
    Arc::new(move |args: &[u8]| {
        let args: Value = serde_json::from_slice(args).unwrap_or(Value::Null);
        let out = f(&args).to_string().into_bytes();
        let run: ActionFuture = Box::pin(async move { Ok(out) });
        Ok(run)
    })
}

/// Puts them on `list`, as the Board's `manifest` declares them.
///
/// # Errors
///
/// One is on it already.
pub fn register(list: &ActionList, manifest: &Manifest) -> Result<(), ActionError> {
    list.register_forwarded(&info(manifest, "LAB_BOARD_STATE"), answer(|_| snapshot()))?;
    list.register_forwarded(
        &info(manifest, "LAB_BOARD_PIN"),
        answer(|args| {
            let what = args
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("something")
                .to_owned();
            PINS.with(|p| p.borrow_mut().push(what.clone()));
            json!({ "pinned": what, "state": snapshot() })
        }),
    )?;
    list.register_forwarded(
        &info(manifest, "LAB_BOARD_CLEAR"),
        answer(|_| {
            PINS.with(|p| p.borrow_mut().clear());
            snapshot()
        }),
    )?;
    Ok(())
}
