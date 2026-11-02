//! `COMMANDS_RUN` as an agent sees it: each command carries its own
//! effect (`COMMANDS_LIST`), a destructive one waits for a person's yes,
//! and an Action List action run through it is authorised as itself.
#![cfg(feature = "ui")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::future_not_send
)]

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, CONFIRM_TTL, Effect};
use architect_commands::{
    Command, CommandList, LIST_ACTION, RUN_ACTION, use_action_list, use_command,
    use_command_actions, use_commands_provider,
};
use architect_permissions::{Action, Decision, PermissionEngine, Principal, Resource};
use dioxus::core::NoOpMutations;
use dioxus::prelude::{Element, VNode, VirtualDom, use_hook};
use serde_json::json;

const fn meta(id: &'static str, label: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: "",
        display_name: label,
        description: "",
        category: "",
        group: "Files",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static PURGE: ActionMeta = meta("FILES_PURGE", "Purge");
static OPEN: ActionMeta = meta("NOTES_OPEN", "Open a note");

static DELETED: AtomicUsize = AtomicUsize::new(0);
static RENAMED: AtomicUsize = AtomicUsize::new(0);
static PURGED: AtomicUsize = AtomicUsize::new(0);
static OPENED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static LIST: RefCell<Option<Arc<ActionList>>> = const { RefCell::new(None) };
}

/// Agents may run anything but `NOTES_OPEN`.
struct NoNotes;
impl PermissionEngine for NoNotes {
    fn check(&self, _: &Principal, _: &Resource, _: &Action) -> Decision {
        Decision::Allow
    }
    fn authorize(&self, _: &Principal, id: &str) -> Decision {
        if id == "NOTES_OPEN" {
            Decision::deny("no notes for agents")
        } else {
            Decision::Allow
        }
    }
}

fn list() -> Arc<ActionList> {
    let list = ActionList::new();
    list.register(&PURGE, Effect::Destructive, |(): ()| {
        PURGED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    list.register(&OPEN, Effect::Mutating, |(): ()| {
        OPENED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .unwrap();
    list.set_authorizer(NoNotes);
    list.require_confirmation(CONFIRM_TTL);
    Arc::new(list)
}

#[allow(non_snake_case)]
fn App() -> Element {
    let _ = use_commands_provider();
    let list = use_hook(list);
    use_action_list(list.clone());
    use_command_actions(list.clone());
    use_command(Command::new("LAB_DELETE", "Delete").destructive(), || {
        DELETED.fetch_add(1, Ordering::SeqCst);
    });
    use_command(Command::new("LAB_RENAME", "Rename"), || {
        RENAMED.fetch_add(1, Ordering::SeqCst);
    });
    use_hook(move || LIST.with(|slot| *slot.borrow_mut() = Some(list)));
    VNode::empty()
}

fn agent() -> ActionCaller {
    ActionCaller::new(Principal::Service {
        name: "agent".to_owned(),
    })
}

/// Runs `COMMANDS_RUN` for `caller`, driving the window's thread until it
/// answers.
async fn run(
    dom: &mut VirtualDom,
    list: &ActionList,
    caller: &ActionCaller,
    id: &str,
) -> Result<Vec<u8>, ActionError> {
    let args = json!({ "id": id }).to_string().into_bytes();
    let mut call = pin!(list.run_json_async_with(caller, RUN_ACTION, &args));
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..500 {
        if let Poll::Ready(out) = call.as_mut().poll(&mut cx) {
            return out;
        }
        let _ = tokio::time::timeout(Duration::from_millis(5), dom.wait_for_work()).await;
        dom.render_immediate(&mut NoOpMutations);
    }
    panic!("COMMANDS_RUN {id} never answered");
}

fn count(n: &AtomicUsize) -> usize {
    n.load(Ordering::SeqCst)
}

/// One test: the counters are process-wide.
#[tokio::test(flavor = "current_thread")]
async fn a_command_runs_with_its_own_effect() {
    let mut dom = VirtualDom::new(App);
    dom.rebuild_in_place();
    let _ = tokio::time::timeout(Duration::from_millis(20), dom.wait_for_work()).await;
    dom.render_immediate(&mut NoOpMutations);
    let list = LIST.with(|slot| slot.borrow_mut().take()).expect("mounted");

    // Every command, with its effect: a view's own and the list's.
    let listed: CommandList =
        facet_json::from_slice(&list.run_json(LIST_ACTION, b"{}").unwrap()).unwrap();
    let effect = |id: &str| {
        listed
            .commands
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.effect.clone())
    };
    assert_eq!(effect("LAB_DELETE").as_deref(), Some("destructive"));
    assert_eq!(effect("LAB_RENAME").as_deref(), Some("mutating"));
    assert_eq!(effect("FILES_PURGE").as_deref(), Some("destructive"));
    assert_eq!(
        list.info(LIST_ACTION).map(|i| i.effect),
        Some(Effect::ReadOnly)
    );

    // A mutating command an agent runs goes ahead.
    run(&mut dom, &list, &agent(), "LAB_RENAME").await.unwrap();
    assert_eq!(count(&RENAMED), 1);

    // A destructive one waits for a person's yes, then runs once.
    let asked = run(&mut dom, &list, &agent(), "LAB_DELETE").await;
    let Err(ActionError::ConfirmationRequired { ticket, .. }) = asked else {
        panic!("a destructive command asks first: {asked:?}");
    };
    assert_eq!(count(&DELETED), 0);
    let waiting = list.pending_confirmations();
    assert_eq!(waiting.len(), 1);
    assert!(waiting[0].args.contains("LAB_DELETE"));
    assert!(list.approve(&ticket));
    run(&mut dom, &list, &agent().with_ticket(ticket), "LAB_DELETE")
        .await
        .unwrap();
    assert_eq!(count(&DELETED), 1);

    // The same for a destructive action run through the runner.
    let asked = run(&mut dom, &list, &agent(), "FILES_PURGE").await;
    assert!(matches!(
        asked,
        Err(ActionError::ConfirmationRequired { .. })
    ));
    assert_eq!(count(&PURGED), 0);

    // The app itself (a key, the palette) is never asked.
    run(&mut dom, &list, &ActionCaller::app(), "LAB_DELETE")
        .await
        .unwrap();
    assert_eq!(count(&DELETED), 2);

    // An action the agent may not run is no nearer through the runner.
    let denied = run(&mut dom, &list, &agent(), "NOTES_OPEN").await;
    let Err(ActionError::Failed { message, .. }) = denied else {
        panic!("the runner asks the authorizer: {denied:?}");
    };
    assert!(message.contains("not allowed"), "{message}");
    assert_eq!(count(&OPENED), 0);
    run(&mut dom, &list, &ActionCaller::app(), "NOTES_OPEN")
        .await
        .unwrap();
    assert_eq!(count(&OPENED), 1);
}
